use crate::{BackendError, GitService, PersistenceService};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

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

fn nonempty(value: Option<&str>) -> &str {
    value
        .filter(|value| !value.is_empty())
        .unwrap_or("*No description provided.*")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResolvedAppPaths;

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
}
