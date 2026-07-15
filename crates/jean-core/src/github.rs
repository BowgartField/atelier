use crate::{
    AdvisoryVulnerability, BackendError, BackendErrorCode, GitHubAuthor, GitHubComment,
    GitHubRepository, GitHubReview, PullRequestContext,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

const MAX_DIFF_SIZE: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssue {
    pub number: u32,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<GitHubLabel>,
    pub created_at: String,
    pub author: GitHubAuthor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssueDetail {
    pub number: u32,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<GitHubLabel>,
    pub created_at: String,
    pub author: GitHubAuthor,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub comments: Vec<GitHubComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubIssueListResult {
    pub issues: Vec<GitHubIssue>,
    pub total_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPullRequest {
    pub number: u32,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub is_draft: bool,
    pub created_at: String,
    pub author: GitHubAuthor,
    #[serde(default)]
    pub labels: Vec<GitHubLabel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubPullRequestDetail {
    pub number: u32,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub is_draft: bool,
    pub created_at: String,
    pub author: GitHubAuthor,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub labels: Vec<GitHubLabel>,
    #[serde(default)]
    pub comments: Vec<GitHubComment>,
    #[serde(default)]
    pub reviews: Vec<GitHubReview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependabotAlert {
    pub number: u32,
    pub state: String,
    pub package_name: String,
    pub package_ecosystem: String,
    pub manifest_path: String,
    pub ghsa_id: String,
    pub cve_id: Option<String>,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub created_at: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryAdvisory {
    pub ghsa_id: String,
    pub cve_id: Option<String>,
    pub summary: String,
    pub description: String,
    pub severity: String,
    pub state: String,
    pub author_login: Option<String>,
    pub created_at: String,
    pub published_at: Option<String>,
    pub html_url: String,
    pub vulnerabilities: Vec<AdvisoryVulnerability>,
}

#[derive(Debug, Deserialize)]
struct DependabotPackageRaw {
    name: String,
    ecosystem: String,
}

#[derive(Debug, Deserialize)]
struct DependabotDependencyRaw {
    package: DependabotPackageRaw,
    manifest_path: String,
}

#[derive(Debug, Deserialize)]
struct SecurityAdvisoryRaw {
    ghsa_id: String,
    cve_id: Option<String>,
    summary: String,
    description: String,
    severity: String,
}

#[derive(Debug, Deserialize)]
struct DependabotAlertRaw {
    number: u32,
    state: String,
    dependency: DependabotDependencyRaw,
    security_advisory: SecurityAdvisoryRaw,
    created_at: String,
    html_url: String,
}

impl From<DependabotAlertRaw> for DependabotAlert {
    fn from(raw: DependabotAlertRaw) -> Self {
        Self {
            number: raw.number,
            state: raw.state,
            package_name: raw.dependency.package.name,
            package_ecosystem: raw.dependency.package.ecosystem,
            manifest_path: raw.dependency.manifest_path,
            ghsa_id: raw.security_advisory.ghsa_id,
            cve_id: raw.security_advisory.cve_id,
            severity: raw.security_advisory.severity,
            summary: raw.security_advisory.summary,
            description: raw.security_advisory.description,
            created_at: raw.created_at,
            html_url: raw.html_url,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AdvisoryVulnerabilityPackageRaw {
    name: String,
    ecosystem: String,
}

#[derive(Debug, Deserialize)]
struct AdvisoryVulnerabilityRaw {
    package: Option<AdvisoryVulnerabilityPackageRaw>,
    vulnerable_version_range: Option<String>,
    patched_versions: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdvisoryAuthorRaw {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryAdvisoryRaw {
    ghsa_id: String,
    cve_id: Option<String>,
    summary: String,
    description: Option<String>,
    severity: Option<String>,
    state: String,
    author: Option<AdvisoryAuthorRaw>,
    #[serde(rename = "publisher")]
    _publisher: Option<AdvisoryAuthorRaw>,
    created_at: String,
    published_at: Option<String>,
    html_url: String,
    vulnerabilities: Vec<AdvisoryVulnerabilityRaw>,
}

impl From<RepositoryAdvisoryRaw> for RepositoryAdvisory {
    fn from(raw: RepositoryAdvisoryRaw) -> Self {
        Self {
            ghsa_id: raw.ghsa_id,
            cve_id: raw.cve_id,
            summary: raw.summary,
            description: raw.description.unwrap_or_default(),
            severity: raw.severity.unwrap_or_else(|| "unknown".to_string()),
            state: raw.state,
            author_login: raw.author.map(|author| author.login),
            created_at: raw.created_at,
            published_at: raw.published_at,
            html_url: raw.html_url,
            vulnerabilities: raw
                .vulnerabilities
                .into_iter()
                .filter_map(|vulnerability| {
                    vulnerability.package.map(|package| AdvisoryVulnerability {
                        package_name: package.name,
                        package_ecosystem: package.ecosystem,
                        vulnerable_version_range: vulnerability.vulnerable_version_range,
                        patched_versions: vulnerability.patched_versions,
                    })
                })
                .collect(),
        }
    }
}

pub type GhRunner =
    Arc<dyn Fn(&str, &[&str]) -> Result<Output, BackendError> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct GitHubService {
    runner: GhRunner,
}

impl Default for GitHubService {
    fn default() -> Self {
        Self {
            runner: Arc::new(native_gh),
        }
    }
}

impl GitHubService {
    pub fn new(runner: GhRunner) -> Self {
        Self { runner }
    }

    pub fn list_labels(&self, project_path: &str) -> Result<Vec<GitHubLabel>, BackendError> {
        let mut labels: Vec<GitHubLabel> = self.run_json(
            project_path,
            &["label", "list", "--json", "name,color", "-L", "1000"],
            "gh label list",
        )?;
        labels.sort_by_key(|label: &GitHubLabel| label.name.to_lowercase());
        Ok(labels)
    }

    pub fn list_issues(
        &self,
        project_path: &str,
        state: Option<&str>,
    ) -> Result<GitHubIssueListResult, BackendError> {
        let state = state.unwrap_or("open");
        let issues: Vec<GitHubIssue> = self.run_json(
            project_path,
            &[
                "issue",
                "list",
                "--json",
                "number,title,body,state,labels,createdAt,author",
                "-L",
                "1000",
                "--state",
                state,
            ],
            "gh issue list",
        )?;
        let total_count = self
            .issue_total_count(project_path, state)
            .unwrap_or(issues.len() as u32);
        Ok(GitHubIssueListResult {
            issues,
            total_count,
        })
    }

    pub fn search_issues(
        &self,
        project_path: &str,
        query: &str,
    ) -> Result<Vec<GitHubIssue>, BackendError> {
        self.run_json(
            project_path,
            &[
                "issue",
                "list",
                "--search",
                query,
                "--json",
                "number,title,body,state,labels,createdAt,author",
                "-L",
                "100",
                "--state",
                "all",
            ],
            "gh issue list --search",
        )
    }

    pub fn issue(&self, project_path: &str, number: u32) -> Result<GitHubIssue, BackendError> {
        self.issue_json(
            project_path,
            number,
            "number,title,body,state,labels,createdAt,author",
        )
    }

    pub fn issue_detail(
        &self,
        project_path: &str,
        number: u32,
    ) -> Result<GitHubIssueDetail, BackendError> {
        self.issue_json(
            project_path,
            number,
            "number,title,body,state,labels,createdAt,author,url,comments",
        )
    }

    pub fn list_pull_requests(
        &self,
        project_path: &str,
        state: Option<&str>,
    ) -> Result<Vec<GitHubPullRequest>, BackendError> {
        let state = state.unwrap_or("open");
        self.run_json(
            project_path,
            &[
                "pr",
                "list",
                "--json",
                "number,title,body,state,headRefName,baseRefName,isDraft,createdAt,author,labels",
                "-L",
                "1000",
                "--state",
                state,
            ],
            "gh pr list",
        )
    }

    pub fn search_pull_requests(
        &self,
        project_path: &str,
        query: &str,
    ) -> Result<Vec<GitHubPullRequest>, BackendError> {
        self.run_json(
            project_path,
            &[
                "pr",
                "list",
                "--search",
                query,
                "--json",
                "number,title,body,state,headRefName,baseRefName,isDraft,createdAt,author,labels",
                "-L",
                "100",
                "--state",
                "all",
            ],
            "gh pr list --search",
        )
    }

    pub fn pull_request(
        &self,
        project_path: &str,
        number: u32,
    ) -> Result<GitHubPullRequest, BackendError> {
        let number_arg = number.to_string();
        let output = (self.runner)(
            project_path,
            &[
                "pr",
                "view",
                &number_arg,
                "--json",
                "number,title,body,state,headRefName,baseRefName,isDraft,createdAt,author,labels",
            ],
        )?;
        if !output.status.success() {
            return Err(gh_pr_failure(output, number));
        }
        parse_json(&output.stdout)
    }

    pub fn pull_request_detail(
        &self,
        project_path: &str,
        number: u32,
    ) -> Result<GitHubPullRequestDetail, BackendError> {
        let number_arg = number.to_string();
        let output = (self.runner)(
            project_path,
            &[
                "pr",
                "view",
                &number_arg,
                "--json",
                "number,title,body,state,headRefName,baseRefName,isDraft,createdAt,author,url,labels,comments,reviews",
            ],
        )?;
        if !output.status.success() {
            return Err(gh_pr_failure(output, number));
        }
        serde_json::from_slice(&output.stdout).map_err(|error| {
            BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Failed to parse gh response: {error}"),
            )
        })
    }

    pub fn pull_request_diff(
        &self,
        project_path: &str,
        number: u32,
    ) -> Result<String, BackendError> {
        let number_arg = number.to_string();
        let output = (self.runner)(
            project_path,
            &["pr", "diff", &number_arg, "--color", "never"],
        )?;
        if !output.status.success() {
            return Ok(String::new());
        }
        let diff = String::from_utf8_lossy(&output.stdout).into_owned();
        if diff.len() <= MAX_DIFF_SIZE {
            return Ok(diff);
        }
        let end = diff
            .char_indices()
            .take_while(|(index, _)| *index < MAX_DIFF_SIZE)
            .last()
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(MAX_DIFF_SIZE.min(diff.len()));
        Ok(format!(
            "{}...\n\n[Diff truncated at 100KB - {} bytes total. Run `gh pr diff {number}` to see the full diff.]",
            &diff[..end],
            diff.len()
        ))
    }

    pub fn pull_request_context(
        &self,
        project_path: &str,
        number: u32,
    ) -> Result<PullRequestContext, BackendError> {
        let detail = self.pull_request_detail(project_path, number)?;
        Ok(PullRequestContext {
            number: detail.number,
            title: detail.title,
            body: detail.body,
            head_ref_name: detail.head_ref_name,
            base_ref_name: detail.base_ref_name,
            comments: detail.comments,
            reviews: detail.reviews,
            diff: Some(self.pull_request_diff(project_path, number)?),
        })
    }

    pub fn list_dependabot_alerts(
        &self,
        project_path: &str,
        repository: &GitHubRepository,
        state: Option<&str>,
    ) -> Result<Vec<DependabotAlert>, BackendError> {
        let endpoint = format!(
            "/repos/{}/{}/dependabot/alerts?state={}&per_page=100",
            repository.owner,
            repository.repo,
            state.unwrap_or("open")
        );
        let output = (self.runner)(project_path, &["api", &endpoint])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("404") || stderr.contains("Dependabot alerts are not available") {
                return Ok(Vec::new());
            }
            return Err(security_api_failure(output, "Dependabot alerts"));
        }
        Ok(parse_json::<Vec<DependabotAlertRaw>>(&output.stdout)?
            .into_iter()
            .map(DependabotAlert::from)
            .collect())
    }

    pub fn dependabot_alert(
        &self,
        project_path: &str,
        repository: &GitHubRepository,
        number: u32,
    ) -> Result<DependabotAlert, BackendError> {
        let endpoint = format!(
            "/repos/{}/{}/dependabot/alerts/{number}",
            repository.owner, repository.repo
        );
        let output = (self.runner)(project_path, &["api", &endpoint])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("404") {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidArgument,
                    format!("Dependabot alert #{number} not found"),
                ));
            }
            return Err(security_api_failure(output, "Dependabot alert"));
        }
        Ok(DependabotAlert::from(parse_json::<DependabotAlertRaw>(
            &output.stdout,
        )?))
    }

    pub fn list_repository_advisories(
        &self,
        project_path: &str,
        repository: &GitHubRepository,
        state: Option<&str>,
    ) -> Result<Vec<RepositoryAdvisory>, BackendError> {
        let mut endpoint = format!(
            "/repos/{}/{}/security-advisories?per_page=100",
            repository.owner, repository.repo
        );
        if let Some(state) = state {
            endpoint.push_str(&format!("&state={state}"));
        }
        let output = (self.runner)(project_path, &["api", &endpoint])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("404") {
                return Ok(Vec::new());
            }
            return Err(security_api_failure(output, "repository advisories"));
        }
        Ok(parse_json::<Vec<RepositoryAdvisoryRaw>>(&output.stdout)?
            .into_iter()
            .map(RepositoryAdvisory::from)
            .collect())
    }

    pub fn repository_advisory(
        &self,
        project_path: &str,
        repository: &GitHubRepository,
        ghsa_id: &str,
    ) -> Result<RepositoryAdvisory, BackendError> {
        let endpoint = format!(
            "/repos/{}/{}/security-advisories/{ghsa_id}",
            repository.owner, repository.repo
        );
        let output = (self.runner)(project_path, &["api", &endpoint])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("404") {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidArgument,
                    format!("Advisory {ghsa_id} not found"),
                ));
            }
            return Err(security_api_failure(output, "repository advisory"));
        }
        Ok(RepositoryAdvisory::from(
            parse_json::<RepositoryAdvisoryRaw>(&output.stdout)?,
        ))
    }

    fn issue_json<T: for<'de> Deserialize<'de>>(
        &self,
        project_path: &str,
        number: u32,
        fields: &str,
    ) -> Result<T, BackendError> {
        let number_arg = number.to_string();
        let output = (self.runner)(
            project_path,
            &["issue", "view", &number_arg, "--json", fields],
        )?;
        if !output.status.success() {
            return Err(gh_issue_failure(output, number));
        }
        parse_json(&output.stdout)
    }

    fn run_json<T: for<'de> Deserialize<'de>>(
        &self,
        project_path: &str,
        args: &[&str],
        operation: &str,
    ) -> Result<T, BackendError> {
        let output = (self.runner)(project_path, args)?;
        if !output.status.success() {
            return Err(gh_failure(output, operation));
        }
        parse_json(&output.stdout)
    }

    fn issue_total_count(&self, project_path: &str, state: &str) -> Option<u32> {
        let repo: serde_json::Value = self
            .run_json(
                project_path,
                &["repo", "view", "--json", "nameWithOwner"],
                "gh repo view",
            )
            .ok()?;
        let name = repo.get("nameWithOwner")?.as_str()?;
        let state_qualifier = match state {
            "closed" => "+state:closed",
            "all" => "",
            _ => "+state:open",
        };
        let query = format!("search/issues?q=repo:{name}+is:issue{state_qualifier}&per_page=1");
        let response: serde_json::Value = self
            .run_json(project_path, &["api", &query], "gh api search/issues")
            .ok()?;
        response
            .get("total_count")?
            .as_u64()
            .map(|count| count as u32)
    }
}

fn security_api_failure(output: Output, operation: &str) -> BackendError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_lowercase();
    let message = if lower.contains("not a git repository") {
        "Not a git repository".to_string()
    } else if lower.contains("not logged")
        || lower.contains("authentication")
        || lower.contains("gh auth login")
    {
        "GitHub CLI not authenticated. Run 'gh auth login' first.".to_string()
    } else if stderr.contains("403") {
        format!("Insufficient permissions to access {operation}.")
    } else {
        format!("gh api failed: {stderr}")
    };
    BackendError::new(BackendErrorCode::Io, message)
}

fn parse_json<T: for<'de> Deserialize<'de>>(stdout: &[u8]) -> Result<T, BackendError> {
    serde_json::from_slice(stdout).map_err(|error| {
        BackendError::new(
            BackendErrorCode::InvalidArgument,
            format!("Failed to parse gh response: {error}"),
        )
    })
}

fn native_gh(project_path: &str, args: &[&str]) -> Result<Output, BackendError> {
    let mut command = Command::new("gh");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .args(args)
        .current_dir(Path::new(project_path))
        .output()
        .map_err(|error| {
            BackendError::new(BackendErrorCode::Io, format!("Failed to run gh: {error}"))
        })
}

fn gh_failure(output: Output, operation: &str) -> BackendError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lower = stderr.to_lowercase();
    let message = if is_unsupported_repository_error(&lower) {
        if lower.contains("not a git repository") {
            "Not a git repository".to_string()
        } else {
            "Could not resolve repository. Is this a GitHub repository?".to_string()
        }
    } else if lower.contains("gh auth login")
        || lower.contains("not authenticated")
        || lower.contains("requires authentication")
        || lower.contains("authentication required")
        || lower.contains("bad credentials")
    {
        "GitHub CLI not authenticated. Run 'gh auth login' first.".to_string()
    } else {
        format!("{operation} failed: {stderr}")
    };
    BackendError::new(BackendErrorCode::Io, message)
}

fn is_unsupported_repository_error(stderr: &str) -> bool {
    stderr.contains("none of the git remotes configured")
        || stderr.contains("no git remotes found")
        || stderr.contains("known github host")
        || stderr.contains("not a github repository")
        || stderr.contains("remote url is not a github repository")
        || stderr.contains("could not resolve repository")
        || stderr.contains("not a git repository")
}

fn gh_issue_failure(output: Output, number: u32) -> BackendError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Could not resolve") || stderr.contains("not found") {
        BackendError::new(
            BackendErrorCode::InvalidArgument,
            format!("Issue #{number} not found"),
        )
    } else {
        gh_failure(output, "gh issue view")
    }
}

fn gh_pr_failure(output: Output, number: u32) -> BackendError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Could not resolve") || stderr.contains("not found") {
        BackendError::new(
            BackendErrorCode::InvalidArgument,
            format!("PR #{number} not found"),
        )
    } else {
        gh_failure(output, "gh pr view")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn successful_output(stdout: &[u8]) -> Output {
        let mut output = Command::new("git").arg("--version").output().unwrap();
        output.stdout = stdout.to_vec();
        output
    }

    #[test]
    fn pull_request_context_uses_one_shared_gh_pipeline() {
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let observed = calls.clone();
        let runner: GhRunner = Arc::new(move |_path, args| {
            observed
                .lock()
                .unwrap()
                .push(args.iter().map(|arg| (*arg).to_string()).collect());
            let stdout = if args.get(1) == Some(&"view") {
                br#"{"number":42,"title":"Shared PR","body":"body","state":"OPEN","headRefName":"feature","baseRefName":"main","isDraft":false,"createdAt":"2026-01-01","author":{"login":"octo"},"comments":[],"reviews":[]}"#.as_slice()
            } else {
                b"diff --git a/file b/file\n".as_slice()
            };
            Ok(successful_output(stdout))
        });

        let context = GitHubService::new(runner)
            .pull_request_context("/repo", 42)
            .unwrap();

        assert_eq!(context.number, 42);
        assert_eq!(context.head_ref_name, "feature");
        assert_eq!(context.diff.as_deref(), Some("diff --git a/file b/file\n"));
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(&calls[0][..3], ["pr", "view", "42"]);
        assert_eq!(&calls[1][..3], ["pr", "diff", "42"]);
    }

    #[test]
    fn issue_listing_and_count_share_the_injected_runner() {
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let observed = calls.clone();
        let runner: GhRunner = Arc::new(move |_path, args| {
            observed
                .lock()
                .unwrap()
                .push(args.iter().map(|arg| (*arg).to_string()).collect());
            let stdout = match args.first().copied() {
                Some("issue") => br#"[{"number":7,"title":"Core issue","body":null,"state":"OPEN","labels":[],"createdAt":"2026-01-01","author":{"login":"octo"}}]"#.as_slice(),
                Some("repo") => br#"{"nameWithOwner":"atelier/jean"}"#.as_slice(),
                Some("api") => br#"{"total_count":321}"#.as_slice(),
                _ => unreachable!(),
            };
            Ok(successful_output(stdout))
        });

        let result = GitHubService::new(runner)
            .list_issues("/repo", Some("closed"))
            .unwrap();

        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.total_count, 321);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(&calls[0][..2], ["issue", "list"]);
        assert_eq!(calls[0].last().map(String::as_str), Some("closed"));
        assert_eq!(&calls[1][..2], ["repo", "view"]);
        assert!(calls[2][1].contains("repo:atelier/jean+is:issue+state:closed"));
    }

    #[test]
    fn list_and_search_operations_are_normalized_in_core() {
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let observed = calls.clone();
        let runner: GhRunner = Arc::new(move |_path, args| {
            observed
                .lock()
                .unwrap()
                .push(args.iter().map(|arg| (*arg).to_string()).collect());
            let stdout = match (args.first().copied(), args.get(1).copied()) {
                (Some("label"), _) => br#"[{"name":"zeta","color":"fff"},{"name":"Alpha","color":"000"}]"#.as_slice(),
                (Some("pr"), Some("list")) => br#"[{"number":9,"title":"Shared PR","body":null,"state":"OPEN","headRefName":"feature","baseRefName":"main","isDraft":false,"createdAt":"2026-01-01","author":{"login":"octo"},"labels":[]}]"#.as_slice(),
                (Some("issue"), Some("view")) => br#"{"number":7,"title":"Core issue","body":null,"state":"OPEN","labels":[],"createdAt":"2026-01-01","author":{"login":"octo"}}"#.as_slice(),
                _ => unreachable!(),
            };
            Ok(successful_output(stdout))
        });
        let service = GitHubService::new(runner);

        let labels = service.list_labels("/repo").unwrap();
        assert_eq!(labels[0].name, "Alpha");
        let prs = service
            .search_pull_requests("/repo", "author:octo")
            .unwrap();
        assert_eq!(prs[0].number, 9);
        assert_eq!(service.issue("/repo", 7).unwrap().title, "Core issue");

        let calls = calls.lock().unwrap();
        assert_eq!(calls[1][2..4], ["--search", "author:octo"]);
        assert_eq!(&calls[2][..3], ["issue", "view", "7"]);
    }

    #[test]
    fn security_reads_share_the_injected_gh_api_pipeline() {
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed = calls.clone();
        let runner: GhRunner = Arc::new(move |_path, args| {
            let endpoint = args.get(1).copied().unwrap_or_default();
            observed.lock().unwrap().push(endpoint.to_string());
            let dependabot = br#"{"number":7,"state":"open","dependency":{"package":{"name":"lodash","ecosystem":"npm"},"manifest_path":"package.json"},"security_advisory":{"ghsa_id":"GHSA-test","cve_id":"CVE-2026-1","summary":"Prototype pollution","description":"details","severity":"high"},"created_at":"2026-01-01","html_url":"https://github.com/acme/widget/security/dependabot/7"}"#;
            let advisory = br#"{"ghsa_id":"GHSA-abcd-1234-5678","cve_id":"CVE-2026-2","summary":"Private advisory","description":"details","severity":"critical","state":"published","author":{"login":"octo"},"publisher":null,"created_at":"2026-01-01","published_at":"2026-01-02","html_url":"https://github.com/acme/widget/security/advisories/GHSA-abcd-1234-5678","vulnerabilities":[{"package":{"name":"crate","ecosystem":"rust"},"vulnerable_version_range":"< 2","patched_versions":"2.0"}]}"#;
            let stdout = if endpoint.contains("dependabot") {
                if endpoint.contains("/7") {
                    dependabot.to_vec()
                } else {
                    [b"[".as_slice(), dependabot, b"]".as_slice()].concat()
                }
            } else if endpoint.ends_with("GHSA-abcd-1234-5678") {
                advisory.to_vec()
            } else {
                [b"[".as_slice(), advisory, b"]".as_slice()].concat()
            };
            Ok(successful_output(&stdout))
        });
        let service = GitHubService::new(runner);
        let repository = GitHubRepository {
            owner: "acme".to_string(),
            repo: "widget".to_string(),
        };

        assert_eq!(
            service
                .list_dependabot_alerts("/repo", &repository, Some("open"))
                .unwrap()[0]
                .package_name,
            "lodash"
        );
        assert_eq!(
            service
                .dependabot_alert("/repo", &repository, 7)
                .unwrap()
                .severity,
            "high"
        );
        assert_eq!(
            service
                .list_repository_advisories("/repo", &repository, Some("published"))
                .unwrap()[0]
                .vulnerabilities[0]
                .patched_versions
                .as_deref(),
            Some("2.0")
        );
        assert_eq!(
            service
                .repository_advisory("/repo", &repository, "GHSA-abcd-1234-5678")
                .unwrap()
                .author_login
                .as_deref(),
            Some("octo")
        );
        let calls = calls.lock().unwrap();
        assert!(calls[0].contains("state=open"));
        assert!(calls[2].contains("state=published"));
    }
}
