use crate::{
    BackendError, BackendErrorCode, GitHubService, GitService, LinearIssueDetail, LinearService,
    PersistenceService,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAuthor {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubComment {
    pub body: String,
    pub author: GitHubAuthor,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubReview {
    pub body: String,
    pub state: String,
    pub author: GitHubAuthor,
    pub submitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueContext {
    pub number: u32,
    pub title: String,
    pub body: Option<String>,
    pub comments: Vec<GitHubComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestContext {
    pub number: u32,
    pub title: String,
    pub body: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub comments: Vec<GitHubComment>,
    pub reviews: Vec<GitHubReview>,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityAlertContext {
    pub number: u32,
    pub package_name: String,
    pub package_ecosystem: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub ghsa_id: String,
    pub cve_id: Option<String>,
    pub manifest_path: String,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryVulnerability {
    pub package_name: String,
    pub package_ecosystem: String,
    pub vulnerable_version_range: Option<String>,
    pub patched_versions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvisoryContext {
    pub ghsa_id: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub cve_id: Option<String>,
    pub vulnerabilities: Vec<AdvisoryVulnerability>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearUser {
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearComment {
    pub body: String,
    pub user: Option<LinearUser>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueContext {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub comments: Vec<LinearComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextRef {
    #[serde(alias = "worktrees")]
    pub sessions: Vec<String>,
    pub orphaned_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextReferences {
    #[serde(default)]
    pub issues: HashMap<String, ContextRef>,
    #[serde(default)]
    pub prs: HashMap<String, ContextRef>,
    #[serde(default)]
    pub security: HashMap<String, ContextRef>,
    #[serde(default)]
    pub advisories: HashMap<String, ContextRef>,
    #[serde(default)]
    pub linear: HashMap<String, ContextRef>,
}

#[derive(Debug, Clone, Default)]
pub struct WorktreeContexts {
    pub issue: Option<IssueContext>,
    pub pull_request: Option<PullRequestContext>,
    pub security: Option<SecurityAlertContext>,
    pub advisory: Option<AdvisoryContext>,
    pub linear: Option<LinearIssueContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedIssueContext {
    pub number: u32,
    pub title: String,
    pub comment_count: usize,
    pub repo_owner: String,
    pub repo_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedPullRequestContext {
    pub number: u32,
    pub title: String,
    pub comment_count: usize,
    pub review_count: usize,
    pub repo_owner: String,
    pub repo_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedLinearIssueContext {
    pub identifier: String,
    pub title: String,
    pub comment_count: usize,
    pub project_name: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedSecurityAlertContext {
    pub number: u32,
    pub package_name: String,
    pub severity: String,
    pub summary: String,
    pub repo_owner: String,
    pub repo_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedAdvisoryContext {
    pub ghsa_id: String,
    pub severity: String,
    pub summary: String,
    pub repo_owner: String,
    pub repo_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueContextContent {
    pub identifier: String,
    pub title: String,
    pub content: String,
}

pub fn slugify_issue_title(title: &str) -> String {
    let slug = title
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == ' ' {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    truncate_chars(&slug, 40).trim_end_matches('-').to_string()
}

pub fn generate_branch_name_from_issue(issue_number: u32, title: &str) -> String {
    format!("issue-{issue_number}-{}", slugify_issue_title(title))
}

pub fn generate_branch_name_from_pr(pr_number: u32, title: &str) -> String {
    format!("pr-{pr_number}-{}", slugify_issue_title(title))
}

pub fn generate_branch_name_from_security_alert(
    alert_number: u32,
    package_name: &str,
    summary: &str,
) -> String {
    let package = package_name.replace('/', "-").replace('@', "");
    let package = truncate_chars(&package, 20);
    format!(
        "security-{alert_number}-{package}-{}",
        slugify_issue_title(summary)
    )
}

pub fn generate_branch_name_from_advisory(ghsa_id: &str, summary: &str) -> String {
    let slug = summary
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = truncate_chars(&slug, 40).trim_end_matches('-');
    let ghsa = ghsa_id.strip_prefix("GHSA-").unwrap_or(ghsa_id);
    format!("advisory-{ghsa}-{slug}")
}

pub fn generate_branch_name_from_linear_issue(identifier: &str, title: &str) -> String {
    format!(
        "linear-{}-{}",
        identifier.to_lowercase(),
        slugify_issue_title(title)
    )
}

fn truncate_chars(value: &str, limit: usize) -> &str {
    value
        .char_indices()
        .nth(limit)
        .map_or(value, |(index, _)| &value[..index])
}

#[derive(Clone)]
pub struct ContextService {
    persistence: Arc<PersistenceService>,
    git: GitService,
    pr_diff_loader: PrDiffLoader,
}

pub type PrDiffLoader =
    Arc<dyn Fn(&str, u32) -> Result<String, BackendError> + Send + Sync + 'static>;

impl ContextService {
    pub fn new(persistence: Arc<PersistenceService>, git: GitService) -> Self {
        Self {
            persistence,
            git,
            pr_diff_loader: Arc::new(native_pr_diff),
        }
    }

    pub fn with_pr_diff_loader(
        persistence: Arc<PersistenceService>,
        git: GitService,
        pr_diff_loader: PrDiffLoader,
    ) -> Self {
        Self {
            persistence,
            git,
            pr_diff_loader,
        }
    }

    pub fn write_worktree_contexts(
        &self,
        project_path: &str,
        project_name: &str,
        worktree_id: &str,
        contexts: &WorktreeContexts,
    ) -> Result<(), BackendError> {
        let directory = self.persistence.git_contexts_dir()?;
        let repository_key = self.git.repository_key(project_path).ok();

        if let (Some(context), Some(repository_key)) = (&contexts.issue, &repository_key) {
            std::fs::write(
                directory.join(format!("{repository_key}-issue-{}.md", context.number)),
                format_issue_context_markdown(context),
            )?;
            self.add_reference(
                "issues",
                format!("{repository_key}-{}", context.number),
                worktree_id,
            )?;
        }
        if let (Some(context), Some(repository_key)) = (&contexts.pull_request, &repository_key) {
            let mut context = context.clone();
            if context.diff.is_none() {
                context.diff = (self.pr_diff_loader)(project_path, context.number).ok();
            }
            std::fs::write(
                directory.join(format!("{repository_key}-pr-{}.md", context.number)),
                format_pr_context_markdown(&context),
            )?;
            self.add_reference(
                "prs",
                format!("{repository_key}-{}", context.number),
                worktree_id,
            )?;
        }
        if let (Some(context), Some(repository_key)) = (&contexts.security, &repository_key) {
            std::fs::write(
                directory.join(format!("{repository_key}-security-{}.md", context.number)),
                format_security_context_markdown(context),
            )?;
            self.add_reference(
                "security",
                format!("{repository_key}-{}", context.number),
                worktree_id,
            )?;
        }
        if let (Some(context), Some(repository_key)) = (&contexts.advisory, &repository_key) {
            std::fs::write(
                directory.join(format!("{repository_key}-advisory-{}.md", context.ghsa_id)),
                format_advisory_context_markdown(context),
            )?;
            self.add_reference(
                "advisories",
                format!("{repository_key}::{}", context.ghsa_id),
                worktree_id,
            )?;
        }
        if let Some(context) = &contexts.linear {
            std::fs::write(
                directory.join(format!(
                    "{project_name}-linear-{}.md",
                    context.identifier.to_lowercase()
                )),
                format_linear_context_markdown(context),
            )?;
            self.add_reference(
                "linear",
                format!("{project_name}-{}", context.identifier),
                worktree_id,
            )?;
        }
        Ok(())
    }

    pub fn load_issue(
        &self,
        github: &GitHubService,
        session_id: &str,
        issue_number: u32,
        project_path: &str,
    ) -> Result<LoadedIssueContext, BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let issue = github.issue_detail(project_path, issue_number)?;
        let context = IssueContext {
            number: issue.number,
            title: issue.title.clone(),
            body: issue.body,
            comments: issue.comments,
        };
        std::fs::write(
            self.persistence
                .git_contexts_dir()?
                .join(format!("{}-issue-{issue_number}.md", repository.key())),
            format_issue_context_markdown(&context),
        )?;
        self.add_reference(
            "issues",
            format!("{}-{issue_number}", repository.key()),
            session_id,
        )?;
        Ok(LoadedIssueContext {
            number: issue.number,
            title: issue.title,
            comment_count: context.comments.len(),
            repo_owner: repository.owner,
            repo_name: repository.repo,
        })
    }

    pub fn list_issues(
        &self,
        session_id: &str,
        worktree_id: Option<&str>,
    ) -> Result<Vec<LoadedIssueContext>, BackendError> {
        let mut contexts = Vec::new();
        for key in self.combined_keys("issues", session_id, worktree_id)? {
            let Some((owner, repo, number)) = parse_numbered_context_key(&key) else {
                continue;
            };
            let path = self
                .persistence
                .git_contexts_dir()?
                .join(format!("{owner}-{repo}-issue-{number}.md"));
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let title = context_title(&content, "# GitHub Issue #")
                .unwrap_or_else(|| format!("Issue #{number}"));
            contexts.push(LoadedIssueContext {
                number,
                title,
                comment_count: content.matches("### @").count(),
                repo_owner: owner,
                repo_name: repo,
            });
        }
        contexts.sort_by_key(|context| context.number);
        Ok(contexts)
    }

    pub fn remove_issue(
        &self,
        session_id: &str,
        issue_number: u32,
        project_path: &str,
    ) -> Result<(), BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let key = format!("{}-{issue_number}", repository.key());
        if self.remove_reference("issues", &key, session_id)? {
            remove_file_if_exists(
                &self
                    .persistence
                    .git_contexts_dir()?
                    .join(format!("{}-issue-{issue_number}.md", repository.key())),
            )?;
        }
        Ok(())
    }

    pub fn issue_content(
        &self,
        session_id: &str,
        issue_number: u32,
        project_path: &str,
    ) -> Result<String, BackendError> {
        self.numbered_content("issues", "issue", session_id, issue_number, project_path)
    }

    pub fn load_pull_request(
        &self,
        github: &GitHubService,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<LoadedPullRequestContext, BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let context = github.pull_request_context(project_path, number)?;
        std::fs::write(
            self.persistence
                .git_contexts_dir()?
                .join(format!("{}-pr-{number}.md", repository.key())),
            format_pr_context_markdown(&context),
        )?;
        self.add_reference("prs", format!("{}-{number}", repository.key()), session_id)?;
        Ok(LoadedPullRequestContext {
            number: context.number,
            title: context.title,
            comment_count: context.comments.len(),
            review_count: context.reviews.len(),
            repo_owner: repository.owner,
            repo_name: repository.repo,
        })
    }

    pub fn list_pull_requests(
        &self,
        session_id: &str,
        worktree_id: Option<&str>,
    ) -> Result<Vec<LoadedPullRequestContext>, BackendError> {
        let mut contexts = Vec::new();
        for key in self.combined_keys("prs", session_id, worktree_id)? {
            let Some((owner, repo, number)) = parse_numbered_context_key(&key) else {
                continue;
            };
            let path = self
                .persistence
                .git_contexts_dir()?
                .join(format!("{owner}-{repo}-pr-{number}.md"));
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let title = context_title(&content, "# GitHub Pull Request #")
                .unwrap_or_else(|| format!("PR #{number}"));
            let comment_count = section_header_count(&content, "## Comments", None);
            let review_count = section_header_count(&content, "## Reviews", Some("## Comments"));
            contexts.push(LoadedPullRequestContext {
                number,
                title,
                comment_count,
                review_count,
                repo_owner: owner,
                repo_name: repo,
            });
        }
        contexts.sort_by_key(|context| context.number);
        Ok(contexts)
    }

    pub fn remove_pull_request(
        &self,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<(), BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let key = format!("{}-{number}", repository.key());
        if self.remove_reference("prs", &key, session_id)? {
            remove_file_if_exists(
                &self
                    .persistence
                    .git_contexts_dir()?
                    .join(format!("{}-pr-{number}.md", repository.key())),
            )?;
        }
        Ok(())
    }

    pub fn pull_request_content(
        &self,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<String, BackendError> {
        self.numbered_content("prs", "pr", session_id, number, project_path)
    }

    pub fn load_security_alert(
        &self,
        github: &GitHubService,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<LoadedSecurityAlertContext, BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let alert = github.dependabot_alert(project_path, &repository, number)?;
        let context = SecurityAlertContext {
            number: alert.number,
            package_name: alert.package_name.clone(),
            package_ecosystem: alert.package_ecosystem,
            severity: alert.severity.clone(),
            summary: alert.summary.clone(),
            description: alert.description,
            ghsa_id: alert.ghsa_id,
            cve_id: alert.cve_id,
            manifest_path: alert.manifest_path,
            html_url: Some(alert.html_url),
        };
        std::fs::write(
            self.persistence
                .git_contexts_dir()?
                .join(format!("{}-security-{number}.md", repository.key())),
            format_security_context_markdown(&context),
        )?;
        self.add_reference(
            "security",
            format!("{}-{number}", repository.key()),
            session_id,
        )?;
        Ok(LoadedSecurityAlertContext {
            number: alert.number,
            package_name: alert.package_name,
            severity: alert.severity,
            summary: alert.summary,
            repo_owner: repository.owner,
            repo_name: repository.repo,
        })
    }

    pub fn list_security_alerts(
        &self,
        session_id: &str,
        worktree_id: Option<&str>,
    ) -> Result<Vec<LoadedSecurityAlertContext>, BackendError> {
        let mut contexts = Vec::new();
        for key in self.combined_keys("security", session_id, worktree_id)? {
            let Some((owner, repo, number)) = parse_numbered_context_key(&key) else {
                continue;
            };
            let path = self
                .persistence
                .git_contexts_dir()?
                .join(format!("{owner}-{repo}-security-{number}.md"));
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let summary = context_title(&content, "# Dependabot Alert #")
                .unwrap_or_else(|| format!("Alert #{number}"));
            let metadata = content.lines().nth(2).unwrap_or_default();
            let severity = metadata_value(metadata, "**Severity:** ", " |")
                .unwrap_or("unknown")
                .to_string();
            let package_name = metadata_value(metadata, "**Package:** ", " (")
                .unwrap_or("unknown")
                .to_string();
            contexts.push(LoadedSecurityAlertContext {
                number,
                package_name,
                severity,
                summary,
                repo_owner: owner,
                repo_name: repo,
            });
        }
        contexts.sort_by_key(|context| context.number);
        Ok(contexts)
    }

    pub fn remove_security_alert(
        &self,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<(), BackendError> {
        let repository = self.git.github_repository(project_path)?;
        if self.remove_reference(
            "security",
            &format!("{}-{number}", repository.key()),
            session_id,
        )? {
            remove_file_if_exists(
                &self
                    .persistence
                    .git_contexts_dir()?
                    .join(format!("{}-security-{number}.md", repository.key())),
            )?;
        }
        Ok(())
    }

    pub fn security_alert_content(
        &self,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<String, BackendError> {
        self.numbered_content("security", "security", session_id, number, project_path)
    }

    pub fn security_keys(&self, session_id: &str) -> Result<Vec<String>, BackendError> {
        self.combined_keys("security", session_id, None)
    }

    pub fn load_advisory(
        &self,
        github: &GitHubService,
        session_id: &str,
        ghsa_id: &str,
        project_path: &str,
    ) -> Result<LoadedAdvisoryContext, BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let advisory = github.repository_advisory(project_path, &repository, ghsa_id)?;
        let context = AdvisoryContext {
            ghsa_id: advisory.ghsa_id.clone(),
            severity: advisory.severity.clone(),
            summary: advisory.summary.clone(),
            description: advisory.description,
            cve_id: advisory.cve_id,
            vulnerabilities: advisory.vulnerabilities,
            html_url: Some(advisory.html_url),
        };
        std::fs::write(
            self.persistence.git_contexts_dir()?.join(format!(
                "{}-advisory-{}.md",
                repository.key(),
                context.ghsa_id
            )),
            format_advisory_context_markdown(&context),
        )?;
        self.add_reference(
            "advisories",
            format!("{}::{}", repository.key(), context.ghsa_id),
            session_id,
        )?;
        Ok(LoadedAdvisoryContext {
            ghsa_id: advisory.ghsa_id,
            severity: advisory.severity,
            summary: advisory.summary,
            repo_owner: repository.owner,
            repo_name: repository.repo,
        })
    }

    pub fn list_advisories(
        &self,
        session_id: &str,
        worktree_id: Option<&str>,
    ) -> Result<Vec<LoadedAdvisoryContext>, BackendError> {
        let mut contexts = Vec::new();
        for key in self.combined_keys("advisories", session_id, worktree_id)? {
            let Some((owner, repo, ghsa_id)) = parse_advisory_context_key(&key) else {
                continue;
            };
            let path = self
                .persistence
                .git_contexts_dir()?
                .join(format!("{owner}-{repo}-advisory-{ghsa_id}.md"));
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let summary = context_title(&content, "# Security Advisory ")
                .unwrap_or_else(|| format!("Advisory {ghsa_id}"));
            let severity = content
                .lines()
                .nth(2)
                .and_then(|line| metadata_value(line, "**Severity:** ", " |"))
                .unwrap_or("unknown")
                .to_string();
            contexts.push(LoadedAdvisoryContext {
                ghsa_id,
                severity,
                summary,
                repo_owner: owner,
                repo_name: repo,
            });
        }
        contexts.sort_by(|left, right| left.ghsa_id.cmp(&right.ghsa_id));
        Ok(contexts)
    }

    pub fn remove_advisory(
        &self,
        session_id: &str,
        ghsa_id: &str,
        project_path: &str,
    ) -> Result<(), BackendError> {
        let repository = self.git.github_repository(project_path)?;
        if self.remove_reference(
            "advisories",
            &format!("{}::{ghsa_id}", repository.key()),
            session_id,
        )? {
            remove_file_if_exists(
                &self
                    .persistence
                    .git_contexts_dir()?
                    .join(format!("{}-advisory-{ghsa_id}.md", repository.key())),
            )?;
        }
        Ok(())
    }

    pub fn advisory_content(
        &self,
        session_id: &str,
        worktree_id: Option<&str>,
        ghsa_id: &str,
        project_path: &str,
    ) -> Result<String, BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let expected = format!("{}::{ghsa_id}", repository.key());
        if !self
            .combined_keys("advisories", session_id, worktree_id)?
            .iter()
            .any(|key| key == &expected)
        {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Session does not have advisory {ghsa_id} loaded"),
            ));
        }
        std::fs::read_to_string(
            self.persistence
                .git_contexts_dir()?
                .join(format!("{}-advisory-{ghsa_id}.md", repository.key())),
        )
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Io,
                format!("Failed to read advisory context file: {error}"),
            )
        })
    }

    pub fn advisory_keys(&self, session_id: &str) -> Result<Vec<String>, BackendError> {
        self.combined_keys("advisories", session_id, None)
    }

    pub async fn load_linear_issue(
        &self,
        linear: &LinearService,
        session_id: &str,
        project_id: &str,
        issue_id: &str,
    ) -> Result<LoadedLinearIssueContext, BackendError> {
        let config = linear.config(project_id)?;
        let mut detail = linear.issue(project_id, issue_id).await?;
        let identifier_lower = detail.identifier.to_lowercase();
        let contexts_dir = self.persistence.git_contexts_dir()?;
        cache_linear_context_images(
            &mut detail,
            &config.api_key,
            &contexts_dir
                .join("linear-context-images")
                .join(&config.project_name)
                .join(&identifier_lower),
        )
        .await;
        std::fs::write(
            contexts_dir.join(format!(
                "{}-linear-{identifier_lower}.md",
                config.project_name
            )),
            format_linear_issue_detail_markdown(&detail),
        )?;
        self.add_reference(
            "linear",
            format!("{}-{}", config.project_name, detail.identifier),
            session_id,
        )?;
        Ok(LoadedLinearIssueContext {
            identifier: detail.identifier,
            title: detail.title,
            comment_count: detail.comments.len(),
            project_name: config.project_name,
            url: Some(detail.url),
        })
    }

    pub fn list_linear_issues(
        &self,
        linear: &LinearService,
        session_id: &str,
        worktree_id: Option<&str>,
        project_id: &str,
    ) -> Result<Vec<LoadedLinearIssueContext>, BackendError> {
        let project_name = linear.config(project_id)?.project_name;
        let mut contexts = Vec::new();
        for (identifier, content) in
            self.linear_context_files(&project_name, session_id, worktree_id)?
        {
            let title =
                linear_context_title(&content).unwrap_or_else(|| format!("Issue {identifier}"));
            let comment_count = content
                .split("## Comments")
                .nth(1)
                .map(|section| {
                    section
                        .lines()
                        .filter(|line| line.starts_with("### "))
                        .count()
                })
                .unwrap_or(0);
            let url = content
                .lines()
                .find_map(|line| line.strip_prefix("- **URL**: ").map(str::to_string));
            contexts.push(LoadedLinearIssueContext {
                identifier,
                title,
                comment_count,
                project_name: project_name.clone(),
                url,
            });
        }
        Ok(contexts)
    }

    pub fn linear_issue_contents(
        &self,
        linear: &LinearService,
        session_id: &str,
        worktree_id: Option<&str>,
        project_id: &str,
    ) -> Result<Vec<LinearIssueContextContent>, BackendError> {
        let project_name = linear.config(project_id)?.project_name;
        Ok(self
            .linear_context_files(&project_name, session_id, worktree_id)?
            .into_iter()
            .map(|(identifier, content)| LinearIssueContextContent {
                title: linear_context_title(&content)
                    .unwrap_or_else(|| format!("Issue {identifier}")),
                identifier,
                content,
            })
            .collect())
    }

    pub fn remove_linear_issue(
        &self,
        linear: &LinearService,
        session_id: &str,
        project_id: &str,
        identifier: &str,
    ) -> Result<(), BackendError> {
        let project_name = linear.config(project_id)?.project_name;
        let key = format!("{project_name}-{identifier}");
        if self.remove_reference("linear", &key, session_id)? {
            remove_file_if_exists(&self.persistence.git_contexts_dir()?.join(format!(
                "{project_name}-linear-{}.md",
                identifier.to_lowercase()
            )))?;
        }
        Ok(())
    }

    pub fn linear_keys(&self, session_id: &str) -> Result<Vec<String>, BackendError> {
        self.combined_keys("linear", session_id, None)
    }

    pub fn linear_identifiers(
        &self,
        session_id: &str,
        project_name: &str,
    ) -> Result<Vec<String>, BackendError> {
        let prefix = format!("{project_name}-");
        Ok(self
            .linear_keys(session_id)?
            .into_iter()
            .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
            .collect())
    }

    fn linear_context_files(
        &self,
        project_name: &str,
        session_id: &str,
        worktree_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, BackendError> {
        let prefix = format!("{project_name}-");
        let directory = self.persistence.git_contexts_dir()?;
        Ok(self
            .combined_keys("linear", session_id, worktree_id)?
            .into_iter()
            .filter_map(|key| {
                let identifier = key.strip_prefix(&prefix)?.to_string();
                let path = directory.join(format!(
                    "{project_name}-linear-{}.md",
                    identifier.to_lowercase()
                ));
                std::fs::read_to_string(path)
                    .ok()
                    .map(|content| (identifier, content))
            })
            .collect())
    }

    fn add_reference(
        &self,
        category: &str,
        key: String,
        worktree_id: &str,
    ) -> Result<(), BackendError> {
        self.persistence.update_context_references(|references| {
            let entries = match category {
                "issues" => &mut references.issues,
                "prs" => &mut references.prs,
                "security" => &mut references.security,
                "advisories" => &mut references.advisories,
                "linear" => &mut references.linear,
                _ => unreachable!("known context category"),
            };
            let reference = entries.entry(key).or_default();
            if !reference.sessions.iter().any(|id| id == worktree_id) {
                reference.sessions.push(worktree_id.to_string());
            }
            reference.orphaned_at = None;
            Ok(())
        })
    }

    fn remove_reference(
        &self,
        category: &str,
        key: &str,
        session_id: &str,
    ) -> Result<bool, BackendError> {
        self.persistence.update_context_references(|references| {
            let entries = reference_map_mut(references, category);
            let Some(reference) = entries.get_mut(key) else {
                return Ok(false);
            };
            reference.sessions.retain(|id| id != session_id);
            if reference.sessions.is_empty() && reference.orphaned_at.is_none() {
                reference.orphaned_at = Some(now_seconds());
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }

    fn combined_keys(
        &self,
        category: &str,
        session_id: &str,
        worktree_id: Option<&str>,
    ) -> Result<Vec<String>, BackendError> {
        let references = self.persistence.load_context_references()?;
        let entries = reference_map(&references, category);
        Ok(entries
            .iter()
            .filter(|(_, reference)| {
                reference.sessions.iter().any(|id| id == session_id)
                    || worktree_id.is_some_and(|worktree_id| {
                        reference.sessions.iter().any(|id| id == worktree_id)
                    })
            })
            .map(|(key, _)| key.clone())
            .collect())
    }

    fn numbered_content(
        &self,
        category: &str,
        file_kind: &str,
        session_id: &str,
        number: u32,
        project_path: &str,
    ) -> Result<String, BackendError> {
        let repository = self.git.github_repository(project_path)?;
        let key = format!("{}-{number}", repository.key());
        if !self
            .combined_keys(category, session_id, None)?
            .iter()
            .any(|candidate| candidate == &key)
        {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Session does not have {file_kind} #{number} loaded"),
            ));
        }
        let path = self
            .persistence
            .git_contexts_dir()?
            .join(format!("{}-{file_kind}-{number}.md", repository.key()));
        std::fs::read_to_string(&path).map_err(|error| {
            BackendError::new(
                BackendErrorCode::Io,
                format!("Failed to read {file_kind} context file: {error}"),
            )
        })
    }
}

fn linear_context_title(content: &str) -> Option<String> {
    content
        .lines()
        .next()?
        .strip_prefix("# Linear Issue ")?
        .split_once(": ")
        .map(|(_, title)| title.to_string())
}

fn reference_map<'a>(
    references: &'a ContextReferences,
    category: &str,
) -> &'a HashMap<String, ContextRef> {
    match category {
        "issues" => &references.issues,
        "prs" => &references.prs,
        "security" => &references.security,
        "advisories" => &references.advisories,
        "linear" => &references.linear,
        _ => unreachable!("known context category"),
    }
}

fn reference_map_mut<'a>(
    references: &'a mut ContextReferences,
    category: &str,
) -> &'a mut HashMap<String, ContextRef> {
    match category {
        "issues" => &mut references.issues,
        "prs" => &mut references.prs,
        "security" => &mut references.security,
        "advisories" => &mut references.advisories,
        "linear" => &mut references.linear,
        _ => unreachable!("known context category"),
    }
}

fn parse_numbered_context_key(key: &str) -> Option<(String, String, u32)> {
    let (repository, number) = key.rsplit_once('-')?;
    let (owner, repo) = repository.split_once('-')?;
    Some((owner.to_string(), repo.to_string(), number.parse().ok()?))
}

fn parse_advisory_context_key(key: &str) -> Option<(String, String, String)> {
    let (repository, ghsa_id) = key.split_once("::")?;
    let (owner, repo) = repository.split_once('-')?;
    Some((owner.to_string(), repo.to_string(), ghsa_id.to_string()))
}

fn metadata_value<'a>(line: &'a str, prefix: &str, end: &str) -> Option<&'a str> {
    line.split(prefix)
        .nth(1)
        .map(|value| value.split(end).next().unwrap_or(value).trim())
}

fn context_title(content: &str, prefix: &str) -> Option<String> {
    content
        .lines()
        .next()?
        .strip_prefix(prefix)?
        .split_once(": ")
        .map(|(_, title)| title.to_string())
}

fn section_header_count(content: &str, start: &str, end: Option<&str>) -> usize {
    let Some(start) = content.find(start) else {
        return 0;
    };
    let section = &content[start..];
    let end = end
        .and_then(|marker| section.find(marker))
        .unwrap_or(section.len());
    section[..end].matches("### @").count()
}

fn remove_file_if_exists(path: &Path) -> Result<(), BackendError> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn native_pr_diff(project_path: &str, number: u32) -> Result<String, BackendError> {
    let output = Command::new("gh")
        .current_dir(Path::new(project_path))
        .args(["pr", "diff", &number.to_string(), "--color", "never"])
        .output()?;
    if !output.status.success() {
        return Ok(String::new());
    }
    let mut diff = String::from_utf8_lossy(&output.stdout).into_owned();
    const MAX_DIFF_BYTES: usize = 100 * 1024;
    if diff.len() > MAX_DIFF_BYTES {
        let mut boundary = MAX_DIFF_BYTES;
        while !diff.is_char_boundary(boundary) {
            boundary -= 1;
        }
        diff.truncate(boundary);
        diff.push_str("\n\n[Diff truncated at 100KB]");
    }
    Ok(diff)
}

pub fn format_issue_context_markdown(context: &IssueContext) -> String {
    let mut content = format!(
        "# GitHub Issue #{}: {}\n\n---\n\n## Description\n\n",
        context.number, context.title
    );
    content.push_str(nonempty(context.body.as_deref()));
    content.push_str("\n\n");
    if !context.comments.is_empty() {
        content.push_str("## Comments\n\n");
        for comment in &context.comments {
            content.push_str(&format!(
                "### @{} ({})\n\n{}\n\n---\n\n",
                comment.author.login, comment.created_at, comment.body
            ));
        }
    }
    content.push_str("---\n\n*Investigate this issue and propose a solution.*\n");
    content
}

pub fn format_pr_context_markdown(context: &PullRequestContext) -> String {
    let mut content = format!(
        "# GitHub Pull Request #{}: {}\n\n**Branch:** `{}` → `{}`\n\n---\n\n## Description\n\n",
        context.number, context.title, context.head_ref_name, context.base_ref_name
    );
    content.push_str(nonempty(context.body.as_deref()));
    content.push_str("\n\n");
    if !context.reviews.is_empty() {
        content.push_str("## Reviews\n\n");
        for review in &context.reviews {
            content.push_str(&format!(
                "### @{} - {} ({})\n\n",
                review.author.login,
                review.state,
                review.submitted_at.as_deref().unwrap_or("Unknown date")
            ));
            if !review.body.is_empty() {
                content.push_str(&review.body);
                content.push_str("\n\n");
            }
            content.push_str("---\n\n");
        }
    }
    if !context.comments.is_empty() {
        content.push_str("## Comments\n\n");
        for comment in &context.comments {
            content.push_str(&format!(
                "### @{} ({})\n\n{}\n\n---\n\n",
                comment.author.login, comment.created_at, comment.body
            ));
        }
    }
    if let Some(diff) = context.diff.as_deref().filter(|diff| !diff.is_empty()) {
        content.push_str("## Changes (Diff)\n\n```diff\n");
        content.push_str(diff);
        if !diff.ends_with('\n') {
            content.push('\n');
        }
        content.push_str("```\n\n");
    }
    content.push_str("---\n\n*Review this pull request and provide feedback or make changes.*\n");
    content
}

pub fn format_security_context_markdown(context: &SecurityAlertContext) -> String {
    let mut content = format!(
        "# Dependabot Alert #{}: {}\n\n**Severity:** {} | **Package:** {} ({}) | **Manifest:** {}\n\n**GHSA:** {}",
        context.number,
        context.summary,
        context.severity,
        context.package_name,
        context.package_ecosystem,
        context.manifest_path,
        context.ghsa_id
    );
    if let Some(cve) = &context.cve_id {
        content.push_str(&format!(" | **CVE:** {cve}"));
    }
    content.push_str("\n\n---\n\n## Description\n\n");
    content.push_str(&context.description);
    content.push_str("\n\n---\n\n*Fix this security vulnerability.*\n");
    content
}

pub fn format_advisory_context_markdown(context: &AdvisoryContext) -> String {
    let mut content = format!(
        "# Security Advisory {}: {}\n\n**Severity:** {}",
        context.ghsa_id, context.summary, context.severity
    );
    if let Some(cve) = &context.cve_id {
        content.push_str(&format!(" | **CVE:** {cve}"));
    }
    content.push_str("\n\n");
    if !context.vulnerabilities.is_empty() {
        content.push_str("## Affected Packages\n\n");
        for vulnerability in &context.vulnerabilities {
            content.push_str(&format!(
                "- **{}** ({})",
                vulnerability.package_name, vulnerability.package_ecosystem
            ));
            if let Some(range) = &vulnerability.vulnerable_version_range {
                content.push_str(&format!(" — vulnerable: {range}"));
            }
            if let Some(patched) = &vulnerability.patched_versions {
                content.push_str(&format!(", patched: {patched}"));
            }
            content.push('\n');
        }
        content.push('\n');
    }
    content.push_str("---\n\n## Description\n\n");
    content.push_str(&context.description);
    content.push_str("\n\n---\n\n*Fix this security advisory.*\n");
    content
}

pub fn format_linear_context_markdown(context: &LinearIssueContext) -> String {
    let mut content = format!(
        "# Linear Issue {}: {}\n\n- **Status**: Unknown\n- **Priority**: No priority\n- **URL**: \n\n---\n\n## Description\n\n",
        context.identifier, context.title
    );
    content.push_str(nonempty(context.description.as_deref()));
    content.push_str("\n\n");
    if !context.comments.is_empty() {
        content.push_str("## Comments\n\n");
        for comment in &context.comments {
            let author = comment
                .user
                .as_ref()
                .map(|user| user.display_name.as_str())
                .unwrap_or("Unknown");
            content.push_str(&format!(
                "### {} ({})\n\n{}\n\n---\n\n",
                author, comment.created_at, comment.body
            ));
        }
    }
    content.push_str("---\n\n*Investigate this issue and propose a solution.*\n");
    content
}

pub fn format_linear_issue_detail_markdown(context: &LinearIssueDetail) -> String {
    let mut content = format!(
        "# Linear Issue {}: {}\n\n- **Status**: {}\n- **Priority**: {}\n",
        context.identifier, context.title, context.state.name, context.priority_label
    );
    if !context.labels.is_empty() {
        content.push_str(&format!(
            "- **Labels**: {}\n",
            context
                .labels
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(assignee) = &context.assignee {
        content.push_str(&format!("- **Assignee**: {}\n", assignee.display_name));
    }
    content.push_str(&format!(
        "- **URL**: {}\n\n---\n\n## Description\n\n",
        context.url
    ));
    content.push_str(nonempty(context.description.as_deref()));
    content.push_str("\n\n");
    if !context.comments.is_empty() {
        content.push_str("## Comments\n\n");
        for comment in &context.comments {
            let author = comment
                .user
                .as_ref()
                .map(|user| user.display_name.as_str())
                .unwrap_or("Unknown");
            content.push_str(&format!(
                "### {} ({})\n\n{}\n\n---\n\n",
                author, comment.created_at, comment.body
            ));
        }
    }
    content.push_str("---\n\n*Investigate this issue and propose a solution.*\n");
    content
}

const MAX_LINEAR_CONTEXT_IMAGE_BYTES: usize = 15 * 1024 * 1024;

fn is_trusted_linear_image_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str().is_some_and(|host| {
            host == "uploads.linear.app" || host.ends_with(".uploads.linear.app")
        })
}

fn extract_linear_image_urls(markdown: &str) -> Vec<String> {
    let markdown_images = regex::Regex::new(r"!\[[^\]]*\]\(\s*<?([^)\s>]+)>?\s*\)").unwrap();
    let html_images =
        regex::Regex::new(r#"<img\b[^>]*\bsrc\s*=\s*["']([^"']+)["'][^>]*>"#).unwrap();
    let mut seen = HashSet::new();
    markdown_images
        .captures_iter(markdown)
        .chain(html_images.captures_iter(markdown))
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .filter(|url| is_trusted_linear_image_url(url) && seen.insert(url.clone()))
        .collect()
}

fn rewrite_linear_image_urls(markdown: &str, replacements: &HashMap<String, PathBuf>) -> String {
    let markdown_images = regex::Regex::new(r"!\[([^\]]*)\]\(\s*<?([^)\s>]+)>?\s*\)").unwrap();
    let html_images =
        regex::Regex::new(r#"(<img\b[^>]*\bsrc\s*=\s*["'])([^"']+)(["'][^>]*>)"#).unwrap();
    let rewritten = markdown_images.replace_all(markdown, |captures: &regex::Captures<'_>| {
        let url = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or_default();
        replacements.get(url).map_or_else(
            || captures[0].to_string(),
            |path| format!("![{}](<{}>)", &captures[1], path.to_string_lossy()),
        )
    });
    html_images
        .replace_all(&rewritten, |captures: &regex::Captures<'_>| {
            let url = captures
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or_default();
            replacements.get(url).map_or_else(
                || captures[0].to_string(),
                |path| format!("{}{}{}", &captures[1], path.to_string_lossy(), &captures[3]),
            )
        })
        .into_owned()
}

fn linear_image_extension(url: &str, content_type: Option<&str>) -> &'static str {
    match content_type
        .and_then(|value| value.split(';').next())
        .unwrap_or_default()
    {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ if url.to_lowercase().contains(".png") => "png",
        _ if url.to_lowercase().contains(".jpg") || url.to_lowercase().contains(".jpeg") => "jpg",
        _ if url.to_lowercase().contains(".gif") => "gif",
        _ if url.to_lowercase().contains(".webp") => "webp",
        _ if url.to_lowercase().contains(".svg") => "svg",
        _ => "bin",
    }
}

async fn download_linear_context_image(
    client: &reqwest::Client,
    api_key: &str,
    url: &str,
    cache_dir: &Path,
) -> Result<PathBuf, BackendError> {
    if !is_trusted_linear_image_url(url) {
        return Err(BackendError::new(
            BackendErrorCode::InvalidArgument,
            format!("Refusing untrusted Linear image URL: {url}"),
        ));
    }
    let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
    let mut response = client
        .get(url)
        .header("Authorization", api_key)
        .send()
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Io,
                format!("Failed to download Linear image: {error}"),
            )
        })?;
    if !response.status().is_success() {
        return Err(BackendError::new(
            BackendErrorCode::Io,
            format!(
                "Linear image download failed with status {}",
                response.status()
            ),
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if content_type
        .as_deref()
        .is_some_and(|value| !value.starts_with("image/"))
    {
        return Err(BackendError::new(
            BackendErrorCode::InvalidArgument,
            format!(
                "Linear image URL returned non-image content type: {}",
                content_type.unwrap()
            ),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::Io,
            format!("Failed to read Linear image bytes: {error}"),
        )
    })? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_LINEAR_CONTEXT_IMAGE_BYTES {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Linear image is too large to cache",
            ));
        }
    }
    std::fs::create_dir_all(cache_dir)?;
    let path = cache_dir.join(format!(
        "{hash}.{}",
        linear_image_extension(url, content_type.as_deref())
    ));
    if !path.exists() {
        std::fs::write(&path, bytes)?;
    }
    Ok(path)
}

async fn cache_linear_markdown_images(
    markdown: &str,
    client: &reqwest::Client,
    api_key: &str,
    cache_dir: &Path,
    replacements: &mut HashMap<String, PathBuf>,
) -> String {
    for url in extract_linear_image_urls(markdown) {
        if replacements.contains_key(&url) {
            continue;
        }
        match download_linear_context_image(client, api_key, &url, cache_dir).await {
            Ok(path) => {
                replacements.insert(url, path);
            }
            Err(error) => log::warn!("Failed to cache Linear context image: {error}"),
        }
    }
    rewrite_linear_image_urls(markdown, replacements)
}

async fn cache_linear_context_images(
    detail: &mut LinearIssueDetail,
    api_key: &str,
    cache_dir: &Path,
) {
    let client = reqwest::Client::new();
    let mut replacements = HashMap::new();
    if let Some(description) = &detail.description {
        detail.description = Some(
            cache_linear_markdown_images(
                description,
                &client,
                api_key,
                cache_dir,
                &mut replacements,
            )
            .await,
        );
    }
    for comment in &mut detail.comments {
        comment.body = cache_linear_markdown_images(
            &comment.body,
            &client,
            api_key,
            cache_dir,
            &mut replacements,
        )
        .await;
    }
}

fn nonempty(value: Option<&str>) -> &str {
    value
        .filter(|value| !value.is_empty())
        .unwrap_or("*No description provided.*")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GhRunner, LinearTransport, ProjectsSnapshot, ResolvedAppPaths};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::process::Output;

    struct LinearIssueTransport;

    #[async_trait]
    impl LinearTransport for LinearIssueTransport {
        async fn graphql(
            &self,
            _api_key: &str,
            _query: &str,
            _variables: Option<Value>,
        ) -> Result<Value, BackendError> {
            Ok(serde_json::json!({"data":{"issue":{
                "id":"issue-id", "identifier":"ENG-42", "title":"Shared Linear",
                "description":"body", "state":{"name":"Todo","type":"unstarted","color":"#fff"},
                "labels":{"nodes":[{"name":"bug","color":"#f00"}]},
                "assignee":{"name":"octo","displayName":"Octo"}, "createdAt":"2026-01-01",
                "url":"https://linear.app/issue/ENG-42", "priority":2, "priorityLabel":"High",
                "comments":{"nodes":[{"body":"comment","user":null,"createdAt":"2026-01-02"}]}
            }}}))
        }
    }

    fn successful_output(stdout: &[u8]) -> Output {
        let mut output = Command::new("git").arg("--version").output().unwrap();
        output.stdout = stdout.to_vec();
        output
    }

    #[test]
    fn context_types_keep_the_frontend_camel_case_contract() {
        let context = PullRequestContext {
            number: 42,
            title: "Shared".to_string(),
            body: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            comments: vec![],
            reviews: vec![],
            diff: None,
        };
        let value = serde_json::to_value(context).unwrap();
        assert_eq!(value["headRefName"], "feature");
        assert!(value.get("head_ref_name").is_none());
    }

    #[test]
    fn linear_image_rewriting_only_accepts_trusted_uploads() {
        let markdown = concat!(
            "![one](https://uploads.linear.app/a.png)\n",
            "<img src=\"https://uploads.linear.app/b.jpg\" />\n",
            "![skip](https://example.com/not-linear.png)\n",
            "![skip](http://uploads.linear.app/insecure.png)\n"
        );
        assert_eq!(
            extract_linear_image_urls(markdown),
            vec![
                "https://uploads.linear.app/a.png".to_string(),
                "https://uploads.linear.app/b.jpg".to_string(),
            ]
        );
        let replacements = HashMap::from([
            (
                "https://uploads.linear.app/a.png".to_string(),
                PathBuf::from("/tmp/linear/a.png"),
            ),
            (
                "https://uploads.linear.app/b.jpg".to_string(),
                PathBuf::from("/tmp/linear/b.jpg"),
            ),
        ]);
        assert_eq!(
            rewrite_linear_image_urls(markdown, &replacements),
            concat!(
                "![one](</tmp/linear/a.png>)\n",
                "<img src=\"/tmp/linear/b.jpg\" />\n",
                "![skip](https://example.com/not-linear.png)\n",
                "![skip](http://uploads.linear.app/insecure.png)\n"
            )
        );
    }

    #[test]
    fn context_service_writes_shared_files_and_atomic_references() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["init"])
            .output()
            .unwrap()
            .status
            .success());
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["remote", "add", "origin", "git@github.com:acme/widget.git"])
            .output()
            .unwrap()
            .status
            .success());
        let persistence = Arc::new(PersistenceService::new(Arc::new(ResolvedAppPaths::new(
            temp.path().join("data"),
            temp.path().join("config"),
            temp.path().join("cache"),
            temp.path().join("resources"),
        ))));
        let service = ContextService::new(persistence.clone(), GitService::default());
        service
            .write_worktree_contexts(
                repo.to_str().unwrap(),
                "Widget",
                "worktree-1",
                &WorktreeContexts {
                    issue: Some(IssueContext {
                        number: 12,
                        title: "Fix shared context".to_string(),
                        body: None,
                        comments: vec![],
                    }),
                    linear: Some(LinearIssueContext {
                        id: "linear-id".to_string(),
                        identifier: "ENG-9".to_string(),
                        title: "Linear context".to_string(),
                        description: None,
                        comments: vec![],
                    }),
                    ..WorktreeContexts::default()
                },
            )
            .unwrap();
        let directory = persistence.git_contexts_dir().unwrap();
        assert!(directory.join("acme-widget-issue-12.md").exists());
        assert!(directory.join("Widget-linear-eng-9.md").exists());
        let references: ContextReferences =
            serde_json::from_slice(&std::fs::read(directory.join("references.json")).unwrap())
                .unwrap();
        assert_eq!(
            references.issues["acme-widget-12"].sessions,
            vec!["worktree-1"]
        );
        assert_eq!(
            references.linear["Widget-ENG-9"].sessions,
            vec!["worktree-1"]
        );
    }

    #[test]
    fn issue_and_pr_context_lifecycle_is_shared() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(["init"])
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(["remote", "add", "origin", "git@github.com:acme/widget.git"])
            .output()
            .unwrap()
            .status
            .success());
        let persistence = Arc::new(PersistenceService::new(Arc::new(ResolvedAppPaths::new(
            temp.path().join("data"),
            temp.path().join("config"),
            temp.path().join("cache"),
            temp.path().join("resources"),
        ))));
        let service = ContextService::new(persistence.clone(), GitService::default());
        let runner: GhRunner = Arc::new(|_, args| {
            let stdout = match (args.first().copied(), args.get(1).copied()) {
                (Some("issue"), Some("view")) => br#"{"number":12,"title":"Shared issue","body":"body","state":"OPEN","labels":[],"createdAt":"2026-01-01","author":{"login":"octo"},"comments":[{"body":"hello","author":{"login":"octo"},"createdAt":"2026-01-02"}]}"#.as_slice(),
                (Some("pr"), Some("view")) => br#"{"number":42,"title":"Shared PR","body":"body","state":"OPEN","headRefName":"feature","baseRefName":"main","isDraft":false,"createdAt":"2026-01-01","author":{"login":"octo"},"comments":[{"body":"comment","author":{"login":"octo"},"createdAt":"2026-01-02"}],"reviews":[{"body":"review","state":"APPROVED","author":{"login":"reviewer"},"submittedAt":"2026-01-03"}]}"#.as_slice(),
                (Some("pr"), Some("diff")) => b"diff --git a/file b/file\n".as_slice(),
                (Some("api"), Some(endpoint)) if endpoint.contains("dependabot") => br#"{"number":7,"state":"open","dependency":{"package":{"name":"lodash","ecosystem":"npm"},"manifest_path":"package.json"},"security_advisory":{"ghsa_id":"GHSA-test","cve_id":"CVE-2026-1","summary":"Prototype pollution","description":"details","severity":"high"},"created_at":"2026-01-01","html_url":"https://github.com/acme/widget/security/dependabot/7"}"#.as_slice(),
                (Some("api"), Some(endpoint)) if endpoint.contains("security-advisories") => br#"{"ghsa_id":"GHSA-abcd-1234-5678","cve_id":"CVE-2026-2","summary":"Private advisory","description":"details","severity":"critical","state":"published","author":{"login":"octo"},"publisher":null,"created_at":"2026-01-01","published_at":"2026-01-02","html_url":"https://github.com/acme/widget/security/advisories/GHSA-abcd-1234-5678","vulnerabilities":[{"package":{"name":"crate","ecosystem":"rust"},"vulnerable_version_range":"< 2","patched_versions":"2.0"}]}"#.as_slice(),
                _ => unreachable!(),
            };
            Ok(successful_output(stdout))
        });
        let github = GitHubService::new(runner);
        let path = repo.to_str().unwrap();

        let issue = service.load_issue(&github, "session", 12, path).unwrap();
        let pr = service
            .load_pull_request(&github, "session", 42, path)
            .unwrap();
        let security = service
            .load_security_alert(&github, "session", 7, path)
            .unwrap();
        let advisory = service
            .load_advisory(&github, "session", "GHSA-abcd-1234-5678", path)
            .unwrap();
        assert_eq!(issue.comment_count, 1);
        assert_eq!(pr.review_count, 1);
        assert_eq!(security.package_name, "lodash");
        assert_eq!(advisory.severity, "critical");
        assert_eq!(
            service.list_issues("session", None).unwrap()[0].title,
            "Shared issue"
        );
        assert_eq!(
            service.list_pull_requests("session", None).unwrap()[0].comment_count,
            1
        );
        assert!(service
            .issue_content("session", 12, path)
            .unwrap()
            .contains("Shared issue"));
        assert!(service
            .pull_request_content("session", 42, path)
            .unwrap()
            .contains("diff --git"));
        assert_eq!(
            service.list_security_alerts("session", None).unwrap()[0].summary,
            "Prototype pollution"
        );
        assert_eq!(
            service.list_advisories("session", None).unwrap()[0].summary,
            "Private advisory"
        );
        assert!(service
            .security_alert_content("session", 7, path)
            .unwrap()
            .contains("lodash"));
        assert!(service
            .advisory_content("session", None, "GHSA-abcd-1234-5678", path,)
            .unwrap()
            .contains("Private advisory"));

        service.remove_issue("session", 12, path).unwrap();
        service.remove_pull_request("session", 42, path).unwrap();
        service.remove_security_alert("session", 7, path).unwrap();
        service
            .remove_advisory("session", "GHSA-abcd-1234-5678", path)
            .unwrap();
        let directory = persistence.git_contexts_dir().unwrap();
        assert!(!directory.join("acme-widget-issue-12.md").exists());
        assert!(!directory.join("acme-widget-pr-42.md").exists());
        assert!(!directory.join("acme-widget-security-7.md").exists());
        assert!(!directory
            .join("acme-widget-advisory-GHSA-abcd-1234-5678.md")
            .exists());
        assert!(service.issue_content("session", 12, path).is_err());
    }

    #[tokio::test]
    async fn linear_context_lifecycle_is_shared() {
        let temp = tempfile::tempdir().unwrap();
        let persistence = Arc::new(PersistenceService::new(Arc::new(ResolvedAppPaths::new(
            temp.path().join("data"),
            temp.path().join("config"),
            temp.path().join("cache"),
            temp.path().join("resources"),
        ))));
        persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"project", "name":"Jean", "linear_api_key":"key"
                })],
                ..Default::default()
            })
            .unwrap();
        let linear =
            LinearService::with_transport(persistence.clone(), Arc::new(LinearIssueTransport));
        let contexts = ContextService::new(persistence.clone(), GitService::default());

        let loaded = contexts
            .load_linear_issue(&linear, "session", "project", "issue-id")
            .await
            .unwrap();
        assert_eq!(loaded.identifier, "ENG-42");
        assert_eq!(loaded.comment_count, 1);
        assert_eq!(
            contexts
                .list_linear_issues(&linear, "session", None, "project")
                .unwrap()[0]
                .title,
            "Shared Linear"
        );
        let contents = contexts
            .linear_issue_contents(&linear, "session", None, "project")
            .unwrap();
        assert!(contents[0].content.contains("- **Priority**: High"));
        assert_eq!(
            contexts.linear_identifiers("session", "Jean").unwrap(),
            ["ENG-42"]
        );

        contexts
            .remove_linear_issue(&linear, "session", "project", "ENG-42")
            .unwrap();
        assert!(!persistence
            .git_contexts_dir()
            .unwrap()
            .join("Jean-linear-eng-42.md")
            .exists());
        assert!(contexts.linear_keys("session").unwrap().is_empty());
    }
}
