use crate::{BackendError, BackendErrorCode, LinearComment, LinearUser, PersistenceService};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

const LINEAR_API_URL: &str = "https://api.linear.app/graphql";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueState {
    pub name: String,
    #[serde(rename = "type")]
    pub state_type: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub state: LinearIssueState,
    #[serde(default)]
    pub labels: Vec<LinearLabel>,
    pub assignee: Option<LinearUser>,
    pub created_at: String,
    pub url: String,
    pub priority: u32,
    pub priority_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueDetail {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub state: LinearIssueState,
    #[serde(default)]
    pub labels: Vec<LinearLabel>,
    pub assignee: Option<LinearUser>,
    pub created_at: String,
    pub url: String,
    pub priority: u32,
    pub priority_label: String,
    pub comments: Vec<LinearComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueListResult {
    pub issues: Vec<LinearIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearTeam {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug)]
pub struct LinearConfig {
    pub api_key: String,
    pub project_name: String,
    pub team_id: Option<String>,
}

#[async_trait]
pub trait LinearTransport: Send + Sync {
    async fn graphql(
        &self,
        api_key: &str,
        query: &str,
        variables: Option<Value>,
    ) -> Result<Value, BackendError>;
}

#[derive(Default)]
pub struct ReqwestLinearTransport;

#[async_trait]
impl LinearTransport for ReqwestLinearTransport {
    async fn graphql(
        &self,
        api_key: &str,
        query: &str,
        variables: Option<Value>,
    ) -> Result<Value, BackendError> {
        let client = reqwest::Client::new();
        let mut body = serde_json::json!({ "query": query });
        if let Some(variables) = variables {
            body["variables"] = variables;
        }
        let response = client
            .post(LINEAR_API_URL)
            .header("Authorization", api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Io,
                    format!("Linear API request failed: {error}"),
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let message = if status.as_u16() == 401 {
                "Linear API key is invalid. Update it in project settings.".to_string()
            } else {
                format!("Linear API error ({status}): {text}")
            };
            return Err(BackendError::new(BackendErrorCode::Io, message));
        }
        let json: Value = response.json().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Failed to parse Linear response: {error}"),
            )
        })?;
        if let Some(errors) = json.get("errors") {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Linear GraphQL errors: {errors}"),
            ));
        }
        Ok(json)
    }
}

#[derive(Clone)]
pub struct LinearService {
    persistence: Arc<PersistenceService>,
    transport: Arc<dyn LinearTransport>,
}

impl LinearService {
    pub fn new(persistence: Arc<PersistenceService>) -> Self {
        Self::with_transport(persistence, Arc::new(ReqwestLinearTransport))
    }

    pub fn with_transport(
        persistence: Arc<PersistenceService>,
        transport: Arc<dyn LinearTransport>,
    ) -> Self {
        Self {
            persistence,
            transport,
        }
    }

    pub fn config(&self, project_id: &str) -> Result<LinearConfig, BackendError> {
        let snapshot = self.persistence.load_projects()?;
        let project = snapshot
            .projects
            .iter()
            .find(|project| project.get("id").and_then(Value::as_str) == Some(project_id))
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidArgument,
                    format!("Project not found: {project_id}"),
                )
            })?;
        let project_name = string_value(project, "name")
            .unwrap_or_default()
            .to_string();
        let team_id = optional_nonempty(project, "linear_team_id", "linearTeamId");
        let api_key = optional_nonempty(project, "linear_api_key", "linearApiKey")
            .or_else(|| {
                self.persistence.load_preferences().ok().and_then(|prefs| {
                    optional_nonempty(&prefs, "linear_api_key", "linearApiKey")
                })
            })
            .ok_or_else(|| BackendError::new(BackendErrorCode::InvalidArgument, "No Linear API key configured. Add one in Settings → Integrations, or override per-project."))?;
        Ok(LinearConfig {
            api_key,
            project_name,
            team_id,
        })
    }

    pub async fn list_teams(&self, project_id: &str) -> Result<Vec<LinearTeam>, BackendError> {
        let config = self.config(project_id)?;
        let response = self
            .transport
            .graphql(&config.api_key, LIST_TEAMS_QUERY, None)
            .await?;
        let nodes = nodes_at(
            &response,
            &["data", "teams", "nodes"],
            "Unexpected Linear API response format",
        )?;
        Ok(nodes.iter().filter_map(parse_team_node).collect())
    }

    pub async fn list_issues(
        &self,
        project_id: &str,
    ) -> Result<LinearIssueListResult, BackendError> {
        let config = self.config(project_id)?;
        let query = build_list_issues_query(config.team_id.as_deref());
        let variables = config.team_id.map(|id| serde_json::json!({ "teamId": id }));
        let response = self
            .transport
            .graphql(&config.api_key, &query, variables)
            .await?;
        let nodes = nodes_at(
            &response,
            &["data", "issues", "nodes"],
            "Unexpected Linear API response format",
        )?;
        Ok(LinearIssueListResult {
            issues: nodes.iter().filter_map(parse_issue_node).collect(),
        })
    }

    pub async fn search_issues(
        &self,
        project_id: &str,
        text: &str,
    ) -> Result<Vec<LinearIssue>, BackendError> {
        let config = self.config(project_id)?;
        let query = build_search_issues_query(config.team_id.as_deref());
        let mut variables = serde_json::json!({ "query": text });
        if let Some(team_id) = config.team_id {
            variables["teamId"] = Value::String(team_id);
        }
        let response = self
            .transport
            .graphql(&config.api_key, &query, Some(variables))
            .await?;
        let nodes = nodes_at(
            &response,
            &["data", "issueSearch", "nodes"],
            "Unexpected Linear search response format",
        )?;
        Ok(nodes.iter().filter_map(parse_issue_node).collect())
    }

    pub async fn issue(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<LinearIssueDetail, BackendError> {
        let config = self.config(project_id)?;
        let response = self
            .transport
            .graphql(
                &config.api_key,
                GET_ISSUE_QUERY,
                Some(serde_json::json!({ "id": issue_id })),
            )
            .await?;
        let node = response.pointer("/data/issue").ok_or_else(|| {
            BackendError::new(BackendErrorCode::InvalidArgument, "Issue not found")
        })?;
        let issue = parse_issue_node(node).ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Failed to parse Linear issue",
            )
        })?;
        let comments = node
            .pointer("/comments/nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_comment_node)
            .collect();
        Ok(LinearIssueDetail {
            id: issue.id,
            identifier: issue.identifier,
            title: issue.title,
            description: issue.description,
            state: issue.state,
            labels: issue.labels,
            assignee: issue.assignee,
            created_at: issue.created_at,
            url: issue.url,
            priority: issue.priority,
            priority_label: issue.priority_label,
            comments,
        })
    }

    pub async fn issue_by_number(
        &self,
        project_id: &str,
        number: i64,
    ) -> Result<Option<LinearIssue>, BackendError> {
        let config = self.config(project_id)?;
        let query = build_issue_by_number_query(config.team_id.as_deref());
        let mut variables = serde_json::json!({ "number": number });
        if let Some(team_id) = config.team_id {
            variables["teamId"] = Value::String(team_id);
        }
        let response = self
            .transport
            .graphql(&config.api_key, &query, Some(variables))
            .await?;
        let nodes = nodes_at(
            &response,
            &["data", "issues", "nodes"],
            "Unexpected Linear API response format",
        )?;
        Ok(nodes.first().and_then(parse_issue_node))
    }
}

fn string_value<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
fn optional_nonempty(value: &Value, snake: &str, camel: &str) -> Option<String> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}
fn nodes_at<'a>(
    value: &'a Value,
    path: &[&str],
    message: &str,
) -> Result<&'a Vec<Value>, BackendError> {
    path.iter()
        .try_fold(value, |current, key| current.get(key))
        .and_then(Value::as_array)
        .ok_or_else(|| BackendError::new(BackendErrorCode::InvalidArgument, message))
}
fn parse_team_node(node: &Value) -> Option<LinearTeam> {
    Some(LinearTeam {
        id: string_value(node, "id")?.into(),
        name: string_value(node, "name")?.into(),
        key: string_value(node, "key")?.into(),
    })
}
fn parse_comment_node(node: &Value) -> Option<LinearComment> {
    Some(LinearComment {
        body: string_value(node, "body")?.into(),
        user: node.get("user").filter(|v| !v.is_null()).and_then(|u| {
            Some(LinearUser {
                name: string_value(u, "name")?.into(),
                display_name: string_value(u, "displayName")?.into(),
            })
        }),
        created_at: string_value(node, "createdAt")?.into(),
    })
}
fn parse_issue_node(node: &Value) -> Option<LinearIssue> {
    let state = node.get("state")?;
    let labels = node
        .pointer("/labels/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|label| {
            Some(LinearLabel {
                name: string_value(label, "name")?.into(),
                color: string_value(label, "color")?.into(),
            })
        })
        .collect();
    let assignee = node.get("assignee").filter(|v| !v.is_null()).and_then(|u| {
        Some(LinearUser {
            name: string_value(u, "name")?.into(),
            display_name: string_value(u, "displayName")?.into(),
        })
    });
    Some(LinearIssue {
        id: string_value(node, "id")?.into(),
        identifier: string_value(node, "identifier")?.into(),
        title: string_value(node, "title")?.into(),
        description: string_value(node, "description").map(Into::into),
        state: LinearIssueState {
            name: string_value(state, "name")?.into(),
            state_type: string_value(state, "type")?.into(),
            color: string_value(state, "color")?.into(),
        },
        labels,
        assignee,
        created_at: string_value(node, "createdAt")?.into(),
        url: string_value(node, "url")?.into(),
        priority: node.get("priority")?.as_u64()? as u32,
        priority_label: string_value(node, "priorityLabel")
            .unwrap_or("No priority")
            .into(),
    })
}

const ISSUE_FIELDS: &str = "\n id identifier title description state { name type color } labels { nodes { name color } } assignee { name displayName } createdAt url priority priorityLabel\n";
fn build_list_issues_query(team: Option<&str>) -> String {
    let (vars, filter) = if team.is_some() {
        ("($teamId: ID!)", ", team: { id: { eq: $teamId } }")
    } else {
        ("", "")
    };
    format!("query ListIssues{vars} {{ issues(filter: {{ state: {{ type: {{ in: [\"started\", \"unstarted\", \"backlog\", \"triage\"] }} }}{filter} }}, orderBy: updatedAt, first: 100) {{ nodes {{{ISSUE_FIELDS}}} }} }}")
}
fn build_search_issues_query(team: Option<&str>) -> String {
    let (vars, filter) = if team.is_some() {
        (
            "($query: String!, $teamId: ID!)",
            ", team: { id: { eq: $teamId } }",
        )
    } else {
        ("($query: String!)", "")
    };
    format!("query SearchIssues{vars} {{ issueSearch(query: $query, first: 50, filter: {{ state: {{ type: {{ in: [\"started\", \"unstarted\", \"backlog\", \"triage\"] }} }}{filter} }}) {{ nodes {{{ISSUE_FIELDS}}} }} }}")
}
fn build_issue_by_number_query(team: Option<&str>) -> String {
    let (vars, filter) = if team.is_some() {
        (
            "($number: Float!, $teamId: ID!)",
            ", team: { id: { eq: $teamId } }",
        )
    } else {
        ("($number: Float!)", "")
    };
    format!("query GetIssueByNumber{vars} {{ issues(filter: {{ number: {{ eq: $number }}{filter} }}, first: 1) {{ nodes {{{ISSUE_FIELDS}}} }} }}")
}
const LIST_TEAMS_QUERY: &str = "query ListTeams { teams(first: 250) { nodes { id name key } } }";
const GET_ISSUE_QUERY: &str = "query GetIssue($id: String!) { issue(id: $id) { id identifier title description state { name type color } labels { nodes { name color } } assignee { name displayName } createdAt url priority priorityLabel comments { nodes { body user { name displayName } createdAt } } } }";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProjectsSnapshot, ResolvedAppPaths};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeTransport {
        calls: Mutex<Vec<(String, Option<Value>)>>,
    }

    #[async_trait]
    impl LinearTransport for FakeTransport {
        async fn graphql(
            &self,
            api_key: &str,
            query: &str,
            variables: Option<Value>,
        ) -> Result<Value, BackendError> {
            self.calls
                .lock()
                .unwrap()
                .push((format!("{api_key}:{query}"), variables));
            let issue = serde_json::json!({
                "id": "issue-id", "identifier": "ENG-42", "title": "Shared Linear",
                "description": "body", "state": {"name": "Todo", "type": "unstarted", "color": "#fff"},
                "labels": {"nodes": [{"name": "bug", "color": "#f00"}]},
                "assignee": {"name": "octo", "displayName": "Octo"}, "createdAt": "2026-01-01",
                "url": "https://linear.app/issue/ENG-42", "priority": 2, "priorityLabel": "High",
                "comments": {"nodes": [{"body": "comment", "user": null, "createdAt": "2026-01-02"}]}
            });
            if query.contains("ListTeams") {
                Ok(
                    serde_json::json!({"data":{"teams":{"nodes":[{"id":"team","name":"Engineering","key":"ENG"}]}}}),
                )
            } else if query.contains("issue(id:") {
                Ok(serde_json::json!({"data":{"issue":issue}}))
            } else if query.contains("issueSearch") {
                Ok(serde_json::json!({"data":{"issueSearch":{"nodes":[issue]}}}))
            } else {
                Ok(serde_json::json!({"data":{"issues":{"nodes":[issue]}}}))
            }
        }
    }

    fn service() -> (tempfile::TempDir, LinearService, Arc<FakeTransport>) {
        let temp = tempfile::tempdir().unwrap();
        let persistence = Arc::new(PersistenceService::new(Arc::new(ResolvedAppPaths::new(
            temp.path().join("data"),
            temp.path().join("config"),
            temp.path().join("cache"),
            temp.path().join("resources"),
        ))));
        persistence.save_projects(&ProjectsSnapshot {
            projects: vec![serde_json::json!({"id":"project","name":"Jean","linear_team_id":"team","linear_api_key":"project-key"})],
            ..Default::default()
        }).unwrap();
        persistence
            .save_preferences(&serde_json::json!({"linear_api_key":"global-key"}))
            .unwrap();
        let transport = Arc::new(FakeTransport::default());
        let service = LinearService::with_transport(persistence, transport.clone());
        (temp, service, transport)
    }

    #[tokio::test]
    async fn all_linear_reads_use_shared_config_transport_and_parsing() {
        let (_temp, service, transport) = service();
        assert_eq!(service.config("project").unwrap().api_key, "project-key");
        assert_eq!(service.list_teams("project").await.unwrap()[0].key, "ENG");
        assert_eq!(
            service.list_issues("project").await.unwrap().issues[0].identifier,
            "ENG-42"
        );
        assert_eq!(
            service.search_issues("project", "shared").await.unwrap()[0].title,
            "Shared Linear"
        );
        assert_eq!(
            service
                .issue("project", "issue-id")
                .await
                .unwrap()
                .comments
                .len(),
            1
        );
        assert_eq!(
            service
                .issue_by_number("project", 42)
                .await
                .unwrap()
                .unwrap()
                .id,
            "issue-id"
        );

        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 5);
        assert!(calls
            .iter()
            .all(|(query, _)| query.starts_with("project-key:")));
        assert_eq!(calls[1].1.as_ref().unwrap()["teamId"], "team");
        assert_eq!(calls[2].1.as_ref().unwrap()["query"], "shared");
        assert_eq!(calls[4].1.as_ref().unwrap()["number"], 42);
    }
}
