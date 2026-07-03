use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::gh_cli::config::resolve_gh_binary;
use std::path::Path;

fn gh_command(gh: &Path, project_path: &str) -> std::process::Command {
    crate::platform::resolved_cli_command(gh, Some(Path::new(project_path)))
}

// =============================================================================
// GitHub Actions Types
// =============================================================================

/// A single workflow run from `gh run list`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub database_id: u64,
    pub name: String,
    pub display_title: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub event: String,
    pub head_branch: String,
    pub created_at: String,
    pub url: String,
    pub workflow_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunStep {
    pub name: String,
    pub number: u32,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunJob {
    pub database_id: u64,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub steps: Vec<WorkflowRunStep>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowJobDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunDetailsResult {
    pub jobs: Vec<WorkflowRunJob>,
    #[serde(default)]
    pub job_definitions: Vec<WorkflowJobDefinition>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowJobLogLine {
    pub step_name: String,
    pub timestamp: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowRunView {
    jobs: Vec<WorkflowRunJob>,
    workflow_database_id: Option<u64>,
}

/// Result of listing workflow runs, includes failed count for badge display
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunsResult {
    pub runs: Vec<WorkflowRun>,
    pub failed_count: u32,
}

/// List GitHub Actions workflow runs for a repository
///
/// Uses `gh run list` to fetch recent workflow runs.
/// - branch: optional branch name to filter runs (for PR/worktree-specific views)
/// - Returns up to 30 recent runs with a count of failed runs for badge display
#[tauri::command]
pub async fn list_workflow_runs(
    app: AppHandle,
    project_path: String,
    branch: Option<String>,
) -> Result<WorkflowRunsResult, String> {
    log::trace!("Listing workflow runs for {project_path} with branch: {branch:?}");

    let gh = resolve_gh_binary(&app);

    let mut args = vec![
        "run".to_string(),
        "list".to_string(),
        "--json".to_string(),
        "databaseId,name,displayTitle,status,conclusion,event,headBranch,createdAt,url,workflowName"
            .to_string(),
        "-L".to_string(),
        "100".to_string(),
    ];

    if let Some(ref b) = branch {
        args.push("--branch".to_string());
        args.push(b.clone());
    }

    let output = gh_command(&gh, &project_path)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run gh run list: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("gh auth login") || stderr.contains("authentication") {
            return Err("GitHub CLI not authenticated. Run 'gh auth login' first.".to_string());
        }
        if stderr.contains("not a git repository") {
            return Err("Not a git repository".to_string());
        }
        if stderr.contains("Could not resolve") {
            return Err("Could not resolve repository. Is this a GitHub repository?".to_string());
        }
        return Err(format!("gh run list failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let runs: Vec<WorkflowRun> =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse gh response: {e}"))?;

    // Count failures only for the most recent run per workflow.
    // gh returns runs sorted by createdAt desc, so the first run we see
    // for each workflowName is the latest. Only count it if it failed.
    let mut seen_workflows = std::collections::HashSet::new();
    let mut failed_count: u32 = 0;
    for run in &runs {
        if seen_workflows.insert(&run.workflow_name)
            && matches!(
                run.conclusion.as_deref(),
                Some("failure") | Some("startup_failure")
            )
        {
            failed_count += 1;
        }
    }

    log::trace!("Found {} workflow runs ({failed_count} failed)", runs.len());

    Ok(WorkflowRunsResult { runs, failed_count })
}

#[tauri::command]
pub async fn get_workflow_run(
    app: AppHandle,
    project_path: String,
    run_id: u64,
) -> Result<WorkflowRunDetailsResult, String> {
    log::trace!("Loading workflow run details for {project_path} run {run_id}");

    let gh = resolve_gh_binary(&app);
    let args = vec![
        "run".to_string(),
        "view".to_string(),
        run_id.to_string(),
        "--json".to_string(),
        "jobs,workflowDatabaseId".to_string(),
    ];

    let output = gh_command(&gh, &project_path)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run gh run view: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("gh auth login") || stderr.contains("authentication") {
            return Err("GitHub CLI not authenticated. Run 'gh auth login' first.".to_string());
        }
        if stderr.contains("not a git repository") {
            return Err("Not a git repository".to_string());
        }
        if stderr.contains("Could not resolve") {
            return Err("Could not resolve repository. Is this a GitHub repository?".to_string());
        }
        return Err(format!("gh run view failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let run_view: WorkflowRunView =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse gh response: {e}"))?;

    let job_definitions = match run_view.workflow_database_id {
        Some(workflow_id) => load_workflow_job_definitions(&gh, &project_path, workflow_id),
        None => Vec::new(),
    };
    let result = WorkflowRunDetailsResult {
        jobs: run_view.jobs,
        job_definitions,
    };

    log::trace!(
        "Loaded workflow run details for {run_id} with {} jobs",
        result.jobs.len()
    );

    Ok(result)
}

#[tauri::command]
pub async fn get_workflow_job_logs(
    app: AppHandle,
    project_path: String,
    run_id: u64,
    job_id: u64,
) -> Result<Vec<WorkflowJobLogLine>, String> {
    log::trace!("Loading logs for workflow run {run_id} job {job_id}");

    let gh = resolve_gh_binary(&app);
    let output = gh_command(&gh, &project_path)
        .args([
            "run",
            "view",
            &run_id.to_string(),
            "--job",
            &job_id.to_string(),
            "--log",
        ])
        .output()
        .map_err(|e| format!("Failed to load workflow job logs: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to load workflow job logs: {stderr}"));
    }

    Ok(parse_workflow_job_logs(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_workflow_job_logs(output: &str) -> Vec<WorkflowJobLogLine> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let _job_name = parts.next()?;
            let step_name = parts.next()?.to_string();
            let content = parts.next()?.trim_start_matches('\u{feff}');
            let (timestamp, message) = match content.split_once(' ') {
                Some((timestamp, message))
                    if timestamp.contains('T') && timestamp.ends_with('Z') =>
                {
                    (Some(timestamp.to_string()), message.to_string())
                }
                _ => (None, content.to_string()),
            };

            Some(WorkflowJobLogLine {
                step_name,
                timestamp,
                message,
            })
        })
        .collect()
}

fn load_workflow_job_definitions(
    gh: &Path,
    project_path: &str,
    workflow_id: u64,
) -> Vec<WorkflowJobDefinition> {
    let output = match gh_command(gh, project_path)
        .args(["workflow", "view", &workflow_id.to_string(), "--yaml"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            log::warn!(
                "Failed to load workflow graph for {workflow_id}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return Vec::new();
        }
        Err(error) => {
            log::warn!("Failed to load workflow graph for {workflow_id}: {error}");
            return Vec::new();
        }
    };

    parse_workflow_job_definitions(&String::from_utf8_lossy(&output.stdout))
}

fn parse_workflow_job_definitions(yaml: &str) -> Vec<WorkflowJobDefinition> {
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return Vec::new();
    };
    let Some(jobs) = document.get("jobs").and_then(serde_yaml::Value::as_mapping) else {
        return Vec::new();
    };

    jobs.iter()
        .filter_map(|(id, value)| {
            let id = id.as_str()?.to_string();
            let name = value
                .get("name")
                .and_then(serde_yaml::Value::as_str)
                .unwrap_or(&id)
                .to_string();
            let needs = match value.get("needs") {
                Some(serde_yaml::Value::String(need)) => vec![need.clone()],
                Some(serde_yaml::Value::Sequence(needs)) => needs
                    .iter()
                    .filter_map(serde_yaml::Value::as_str)
                    .map(String::from)
                    .collect(),
                _ => Vec::new(),
            };

            Some(WorkflowJobDefinition { id, name, needs })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_run_deserialization() {
        let json = r#"[{
            "databaseId": 123,
            "name": "build",
            "displayTitle": "Fix bug",
            "status": "completed",
            "conclusion": "failure",
            "event": "push",
            "headBranch": "main",
            "createdAt": "2025-01-01T00:00:00Z",
            "url": "https://github.com/owner/repo/actions/runs/123",
            "workflowName": "CI"
        }]"#;

        let runs: Vec<WorkflowRun> = serde_json::from_str(json).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].database_id, 123);
        assert_eq!(runs[0].conclusion.as_deref(), Some("failure"));
        assert_eq!(runs[0].workflow_name, "CI");
    }

    fn make_run(id: u64, workflow: &str, conclusion: Option<&str>) -> WorkflowRun {
        WorkflowRun {
            database_id: id,
            name: "job".into(),
            display_title: format!("run {id}"),
            status: "completed".into(),
            conclusion: conclusion.map(String::from),
            event: "push".into(),
            head_branch: "main".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
            url: format!("https://example.com/{id}"),
            workflow_name: workflow.into(),
        }
    }

    fn count_failed(runs: &[WorkflowRun]) -> u32 {
        let mut seen = std::collections::HashSet::new();
        let mut count: u32 = 0;
        for run in runs {
            if seen.insert(&run.workflow_name)
                && matches!(
                    run.conclusion.as_deref(),
                    Some("failure") | Some("startup_failure")
                )
            {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn test_failed_count_ignores_old_failures_after_success() {
        // CI: latest=success, older=failure → should NOT count
        // Runs are ordered newest-first (like gh CLI output)
        let runs = vec![
            make_run(2, "CI", Some("success")),
            make_run(1, "CI", Some("failure")),
        ];
        assert_eq!(count_failed(&runs), 0);
    }

    #[test]
    fn test_failed_count_counts_latest_failure() {
        // CI: latest=failure → should count
        // Deploy: latest=success → should NOT count
        let runs = vec![
            make_run(4, "CI", Some("failure")),
            make_run(3, "Deploy", Some("success")),
            make_run(2, "CI", Some("success")),
            make_run(1, "Deploy", Some("failure")),
        ];
        assert_eq!(count_failed(&runs), 1);
    }

    #[test]
    fn test_failed_count_in_progress_not_counted() {
        // CI: latest=in_progress (no conclusion) → should NOT count
        let runs = vec![make_run(2, "CI", None), make_run(1, "CI", Some("failure"))];
        assert_eq!(count_failed(&runs), 0);
    }

    #[test]
    fn test_failed_count_multiple_workflows_failing() {
        let runs = vec![
            make_run(3, "CI", Some("failure")),
            make_run(2, "Deploy", Some("startup_failure")),
            make_run(1, "CI", Some("success")),
        ];
        assert_eq!(count_failed(&runs), 2);
    }

    #[test]
    fn test_workflow_runs_result_serialization() {
        let result = WorkflowRunsResult {
            runs: vec![],
            failed_count: 3,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"failedCount\":3"));
        assert!(json.contains("\"runs\":[]"));
    }

    #[test]
    fn test_workflow_run_details_deserialization() {
        let json = r#"{
            "jobs": [{
                "databaseId": 42,
                "name": "build",
                "status": "completed",
                "conclusion": "success",
                "startedAt": "2026-07-03T07:16:52Z",
                "completedAt": "2026-07-03T07:17:00Z",
                "steps": [{
                    "name": "Set up job",
                    "number": 1,
                    "status": "completed",
                    "conclusion": "success",
                    "startedAt": "2026-07-03T07:16:53Z",
                    "completedAt": "2026-07-03T07:16:54Z"
                }],
                "url": "https://github.com/cli/cli/actions/runs/1/job/42"
            }],
            "jobDefinitions": [{
                "id": "build",
                "name": "Build",
                "needs": []
            }]
        }"#;

        let result: WorkflowRunDetailsResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.jobs.len(), 1);
        assert_eq!(result.jobs[0].database_id, 42);
        assert_eq!(result.jobs[0].steps.len(), 1);
        assert_eq!(result.jobs[0].steps[0].name, "Set up job");
        assert_eq!(result.job_definitions[0].id, "build");
    }

    #[test]
    fn test_parse_workflow_job_definitions() {
        let yaml = r#"
name: Build and deploy
on: push
jobs:
  build:
    name: Build app
    runs-on: ubuntu-latest
  deploy:
    name: Deploy
    needs: [build]
    runs-on: ubuntu-latest
"#;

        let definitions = parse_workflow_job_definitions(yaml);
        assert_eq!(definitions.len(), 2);
        assert_eq!(definitions[0].id, "build");
        assert_eq!(definitions[0].name, "Build app");
        assert_eq!(definitions[1].needs, vec!["build"]);
    }

    #[test]
    fn test_parse_workflow_job_logs() {
        let output = "\
build\tCheckout\t2026-07-03T09:00:00.123Z Fetching repository\n\
build\tTests\t2026-07-03T09:01:00Z Running tests\n";

        let logs = parse_workflow_job_logs(output);
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].step_name, "Checkout");
        assert_eq!(
            logs[0].timestamp.as_deref(),
            Some("2026-07-03T09:00:00.123Z")
        );
        assert_eq!(logs[0].message, "Fetching repository");
        assert_eq!(logs[1].step_name, "Tests");
    }
}
