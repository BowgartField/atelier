use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use super::git::get_repo_identifier;
use crate::gh_cli::config::resolve_gh_binary;

pub use jean_core::{
    AdvisoryContext, DependabotAlert, GitHubAuthor, GitHubComment, GitHubIssue, GitHubIssueDetail,
    GitHubIssueListResult, GitHubLabel, GitHubPullRequest, GitHubPullRequestDetail, IssueContext,
    LoadedAdvisoryContext, LoadedIssueContext, LoadedPullRequestContext,
    LoadedSecurityAlertContext, PullRequestContext, RepositoryAdvisory, SecurityAlertContext,
};

fn gh_command(gh: &Path, project_path: &str) -> Command {
    crate::platform::resolved_cli_command(gh, Some(Path::new(project_path)))
}

// =============================================================================
// GitHub Types
// =============================================================================

#[tauri::command]
pub async fn list_github_labels(
    app: AppHandle,
    project_path: String,
) -> Result<Vec<GitHubLabel>, String> {
    crate::backend_runtime::github_service(&app)
        .list_labels(&project_path)
        .map_err(|error| error.to_string())
}

/// Detect gh errors caused by a directory that cannot be resolved to a GitHub
/// repository. Some of these messages include "gh auth login" as a suggestion,
/// but they are not authentication failures.
fn is_unsupported_github_repo_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("none of the git remotes configured")
        || lower.contains("no git remotes found")
        || lower.contains("known github host")
        || lower.contains("not a github repository")
        || lower.contains("remote url is not a github repository")
        || lower.contains("could not resolve repository")
        || lower.contains("not a git repository")
}

fn is_gh_cli_auth_error(stderr: &str) -> bool {
    if is_unsupported_github_repo_error(stderr) {
        return false;
    }

    let lower = stderr.to_lowercase();
    lower.contains("gh auth login")
        || lower.contains("not authenticated")
        || lower.contains("requires authentication")
        || lower.contains("authentication required")
        || lower.contains("bad credentials")
}

/// List GitHub issues for a repository
///
/// Uses `gh issue list` to fetch issues from the repository.
/// - state: "open", "closed", or "all" (default: "open")
/// - Returns up to 100 issues sorted by creation date (newest first)
/// - Includes total_count from GitHub search API for accurate badge display
#[tauri::command]
pub async fn list_github_issues(
    app: AppHandle,
    project_path: String,
    state: Option<String>,
) -> Result<GitHubIssueListResult, String> {
    crate::backend_runtime::github_service(&app)
        .list_issues(&project_path, state.as_deref())
        .map_err(|error| error.to_string())
}

/// Search GitHub issues using GitHub's search syntax
///
/// Uses `gh issue list --search` to query GitHub's search API.
/// This finds issues beyond the default -L 100 limit.
#[tauri::command]
pub async fn search_github_issues(
    app: AppHandle,
    project_path: String,
    query: String,
) -> Result<Vec<GitHubIssue>, String> {
    crate::backend_runtime::github_service(&app)
        .search_issues(&project_path, &query)
        .map_err(|error| error.to_string())
}

/// Get a GitHub issue by number, returning the same type as list_github_issues.
///
/// Uses `gh issue view` to fetch a single issue by exact number.
/// This finds any issue regardless of age or state.
#[tauri::command]
pub async fn get_github_issue_by_number(
    app: AppHandle,
    project_path: String,
    issue_number: u32,
) -> Result<GitHubIssue, String> {
    crate::backend_runtime::github_service(&app)
        .issue(&project_path, issue_number)
        .map_err(|error| error.to_string())
}

/// Get detailed information about a specific GitHub issue
///
/// Uses `gh issue view` to fetch the issue with comments.
#[tauri::command]
pub async fn get_github_issue(
    app: AppHandle,
    project_path: String,
    issue_number: u32,
) -> Result<GitHubIssueDetail, String> {
    crate::backend_runtime::github_service(&app)
        .issue_detail(&project_path, issue_number)
        .map_err(|error| error.to_string())
}

/// Generate a slug from an issue title for branch naming
/// e.g., "Fix the login bug" -> "fix-the-login-bug"
pub fn slugify_issue_title(title: &str) -> String {
    jean_core::slugify_issue_title(title)
}

/// Generate a branch name from an issue
/// e.g., Issue #123 "Fix the login bug" -> "issue-123-fix-the-login-bug"
pub fn generate_branch_name_from_issue(issue_number: u32, title: &str) -> String {
    jean_core::generate_branch_name_from_issue(issue_number, title)
}

/// Format issue context as markdown for the context file
pub fn format_issue_context_markdown(ctx: &IssueContext) -> String {
    jean_core::format_issue_context_markdown(ctx)
}

// =============================================================================
// Shared Context Reference Tracking
// =============================================================================

/// Reference tracking for a single context file (issue or PR)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextRef {
    #[serde(alias = "worktrees")]
    pub sessions: Vec<String>,
    pub orphaned_at: Option<u64>,
}

/// Tracks which sessions reference which shared context files
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextReferences {
    pub issues: std::collections::HashMap<String, ContextRef>,
    pub prs: std::collections::HashMap<String, ContextRef>,
    #[serde(default)]
    pub security: std::collections::HashMap<String, ContextRef>,
    #[serde(default)]
    pub advisories: std::collections::HashMap<String, ContextRef>,
    #[serde(default)]
    pub linear: std::collections::HashMap<String, ContextRef>,
}

/// Get the directory for shared GitHub contexts
pub fn get_github_contexts_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {e}"))?;
    Ok(app_data_dir.join("git-context"))
}

/// Get the path to the references.json file
pub fn get_references_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(get_github_contexts_dir(app)?.join("references.json"))
}

/// Load context references from disk
pub fn load_context_references(app: &tauri::AppHandle) -> Result<ContextReferences, String> {
    let path = get_references_path(app)?;
    if !path.exists() {
        return Ok(ContextReferences::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read references.json: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse references.json: {e}"))
}

/// Save context references to disk
pub fn save_context_references(
    app: &tauri::AppHandle,
    refs: &ContextReferences,
) -> Result<(), String> {
    let dir = get_github_contexts_dir(app)?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create git-context directory: {e}"))?;

    let path = dir.join("references.json");
    let content = serde_json::to_string_pretty(refs)
        .map_err(|e| format!("Failed to serialize references: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("Failed to write references.json: {e}"))
}

/// Add a session reference to an issue context
/// Key format: "{owner}-{repo}-{number}"
pub fn add_issue_reference(
    app: &tauri::AppHandle,
    repo_key: &str,
    issue_number: u32,
    session_id: &str,
) -> Result<(), String> {
    let mut refs = load_context_references(app)?;
    let key = format!("{repo_key}-{issue_number}");

    let entry = refs.issues.entry(key).or_default();
    if !entry.sessions.contains(&session_id.to_string()) {
        entry.sessions.push(session_id.to_string());
    }
    // Clear orphaned status when a reference is added
    entry.orphaned_at = None;

    save_context_references(app, &refs)
}

/// Add a session reference to a PR context
/// Key format: "{owner}-{repo}-{number}"
pub fn add_pr_reference(
    app: &tauri::AppHandle,
    repo_key: &str,
    pr_number: u32,
    session_id: &str,
) -> Result<(), String> {
    let mut refs = load_context_references(app)?;
    let key = format!("{repo_key}-{pr_number}");

    let entry = refs.prs.entry(key).or_default();
    if !entry.sessions.contains(&session_id.to_string()) {
        entry.sessions.push(session_id.to_string());
    }
    // Clear orphaned status when a reference is added
    entry.orphaned_at = None;

    save_context_references(app, &refs)
}

/// Remove a session reference from an issue context
/// Returns true if the context is now orphaned (no more references)
pub fn remove_issue_reference(
    app: &tauri::AppHandle,
    repo_key: &str,
    issue_number: u32,
    session_id: &str,
) -> Result<bool, String> {
    let mut refs = load_context_references(app)?;
    let key = format!("{repo_key}-{issue_number}");

    let orphaned = if let Some(entry) = refs.issues.get_mut(&key) {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            true
        } else {
            false
        }
    } else {
        false
    };

    save_context_references(app, &refs)?;
    Ok(orphaned)
}

/// Remove a session reference from a PR context
/// Returns true if the context is now orphaned (no more references)
pub fn remove_pr_reference(
    app: &tauri::AppHandle,
    repo_key: &str,
    pr_number: u32,
    session_id: &str,
) -> Result<bool, String> {
    let mut refs = load_context_references(app)?;
    let key = format!("{repo_key}-{pr_number}");

    let orphaned = if let Some(entry) = refs.prs.get_mut(&key) {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            true
        } else {
            false
        }
    } else {
        false
    };

    save_context_references(app, &refs)?;
    Ok(orphaned)
}

/// Get all issue keys referenced by a session
/// Returns keys in format "{owner}-{repo}-{number}"
pub fn get_session_issue_refs(
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<Vec<String>, String> {
    let refs = load_context_references(app)?;
    Ok(refs
        .issues
        .iter()
        .filter(|(_, entry)| entry.sessions.contains(&session_id.to_string()))
        .map(|(key, _)| key.clone())
        .collect())
}

/// Get all PR keys referenced by a session
/// Returns keys in format "{owner}-{repo}-{number}"
pub fn get_session_pr_refs(
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<Vec<String>, String> {
    let refs = load_context_references(app)?;
    Ok(refs
        .prs
        .iter()
        .filter(|(_, entry)| entry.sessions.contains(&session_id.to_string()))
        .map(|(key, _)| key.clone())
        .collect())
}

/// Get all security alert keys referenced by a session
/// Returns keys in format "{owner}-{repo}-{number}"
pub fn get_session_security_refs(
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<Vec<String>, String> {
    crate::backend_runtime::context_service(app)?
        .security_keys(session_id)
        .map_err(|error| error.to_string())
}

/// Get all advisory keys referenced by a session
/// Returns keys in format "{repo_key}::{ghsa_id}"
pub fn get_session_advisory_refs(
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<Vec<String>, String> {
    crate::backend_runtime::context_service(app)?
        .advisory_keys(session_id)
        .map_err(|error| error.to_string())
}

/// Parse an advisory context key into (owner, repo, ghsa_id)
/// Key format: "{owner}-{repo}::{ghsa_id}"
fn parse_advisory_context_key(key: &str) -> Option<(String, String, String)> {
    let (repo_key, ghsa_id) = key.split_once("::")?;
    let (owner, repo) = repo_key.split_once('-')?;
    Some((owner.to_string(), repo.to_string(), ghsa_id.to_string()))
}

fn advisory_refs_contain_expected_key(
    session_refs: &[String],
    worktree_refs: Option<&[String]>,
    expected_key: &str,
) -> bool {
    session_refs.iter().any(|key| key == expected_key)
        || worktree_refs
            .map(|refs| refs.iter().any(|key| key == expected_key))
            .unwrap_or(false)
}

/// Extract the number from a context ref key (format: "{owner}-{repo}-{number}")
fn extract_number_from_ref_key(key: &str) -> Option<u32> {
    key.rsplit('-').next()?.parse().ok()
}

/// Get all issue, PR, and security alert numbers referenced by a session
/// Returns (issue_numbers, pr_numbers, security_numbers)
pub fn get_session_context_numbers(
    app: &AppHandle,
    session_id: &str,
) -> Result<(Vec<u32>, Vec<u32>, Vec<u32>), String> {
    let issue_keys = get_session_issue_refs(app, session_id)?;
    let pr_keys = get_session_pr_refs(app, session_id)?;
    let security_keys = get_session_security_refs(app, session_id)?;

    let issue_nums: Vec<u32> = issue_keys
        .iter()
        .filter_map(|k| extract_number_from_ref_key(k))
        .collect();
    let pr_nums: Vec<u32> = pr_keys
        .iter()
        .filter_map(|k| extract_number_from_ref_key(k))
        .collect();
    let security_nums: Vec<u32> = security_keys
        .iter()
        .filter_map(|k| extract_number_from_ref_key(k))
        .collect();

    Ok((issue_nums, pr_nums, security_nums))
}

/// Get all loaded context markdown content for a session
/// Returns concatenated markdown of all issue, PR, and security context files, or empty string if none
pub fn get_session_context_content(
    app: &AppHandle,
    session_id: &str,
    project_path: &str,
) -> Result<String, String> {
    let repo_id = get_repo_identifier(project_path)?;
    let repo_key = repo_id.to_key();
    let contexts_dir = get_github_contexts_dir(app)?;

    let issue_keys = get_session_issue_refs(app, session_id)?;
    let pr_keys = get_session_pr_refs(app, session_id)?;
    let security_keys = get_session_security_refs(app, session_id)?;
    let advisory_keys = get_session_advisory_refs(app, session_id)?;

    if issue_keys.is_empty()
        && pr_keys.is_empty()
        && security_keys.is_empty()
        && advisory_keys.is_empty()
    {
        return Ok(String::new());
    }

    let mut parts: Vec<String> = Vec::new();

    for key in &issue_keys {
        if let Some(number) = extract_number_from_ref_key(key) {
            let file = contexts_dir.join(format!("{repo_key}-issue-{number}.md"));
            if file.exists() {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    parts.push(format!("### Issue #{number}\n\n{content}"));
                }
            }
        }
    }

    for key in &pr_keys {
        if let Some(number) = extract_number_from_ref_key(key) {
            let file = contexts_dir.join(format!("{repo_key}-pr-{number}.md"));
            if file.exists() {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    parts.push(format!("### PR #{number}\n\n{content}"));
                }
            }
        }
    }

    for key in &security_keys {
        if let Some(number) = extract_number_from_ref_key(key) {
            let file = contexts_dir.join(format!("{repo_key}-security-{number}.md"));
            if file.exists() {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    parts.push(format!("### Security Alert #{number}\n\n{content}"));
                }
            }
        }
    }

    for key in &advisory_keys {
        if let Some((owner, repo, ghsa_id)) = parse_advisory_context_key(key) {
            let adv_repo_key = format!("{owner}-{repo}");
            let file = contexts_dir.join(format!("{adv_repo_key}-advisory-{ghsa_id}.md"));
            if file.exists() {
                if let Ok(content) = std::fs::read_to_string(&file) {
                    parts.push(format!("### Advisory {ghsa_id}\n\n{content}"));
                }
            }
        }
    }

    Ok(parts.join("\n\n"))
}

/// Remove all references for a session
/// Returns (orphaned_issue_keys, orphaned_pr_keys, orphaned_security_keys, orphaned_advisory_keys)
pub fn remove_all_session_references(
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<(Vec<String>, Vec<String>, Vec<String>, Vec<String>), String> {
    let mut refs = load_context_references(app)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut orphaned_issues = Vec::new();
    let mut orphaned_prs = Vec::new();
    let mut orphaned_security = Vec::new();
    let mut orphaned_advisories = Vec::new();

    for (key, entry) in refs.issues.iter_mut() {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(now);
            orphaned_issues.push(key.clone());
        }
    }

    for (key, entry) in refs.prs.iter_mut() {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(now);
            orphaned_prs.push(key.clone());
        }
    }

    for (key, entry) in refs.security.iter_mut() {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(now);
            orphaned_security.push(key.clone());
        }
    }

    for (key, entry) in refs.advisories.iter_mut() {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(now);
            orphaned_advisories.push(key.clone());
        }
    }

    save_context_references(app, &refs)?;
    Ok((
        orphaned_issues,
        orphaned_prs,
        orphaned_security,
        orphaned_advisories,
    ))
}

/// Parse a context key into (repo_owner, repo_name, number)
/// Key format: "{owner}-{repo}-{number}"
fn parse_context_key(key: &str) -> Option<(String, String, u32)> {
    // Split from the right to get the number first
    let (repo_key, number_str) = key.rsplit_once('-')?;
    let number = number_str.parse::<u32>().ok()?;

    // Parse repo_key as "owner-repo" - split on first dash only
    let (owner, repo) = repo_key.split_once('-')?;

    Some((owner.to_string(), repo.to_string(), number))
}

/// Clean up orphaned context files older than retention_days
/// Returns the number of files deleted
pub fn cleanup_orphaned_contexts(
    app: &tauri::AppHandle,
    retention_days: u64,
) -> Result<u32, String> {
    let mut refs = load_context_references(app)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let retention_secs = retention_days * 24 * 60 * 60;

    let contexts_dir = get_github_contexts_dir(app)?;
    let mut deleted_count = 0u32;

    // Clean up orphaned issues
    let issues_to_remove: Vec<String> = refs
        .issues
        .iter()
        .filter_map(|(key, entry)| {
            if let Some(orphaned_at) = entry.orphaned_at {
                if orphaned_at + retention_secs < now {
                    return Some(key.clone());
                }
            }
            None
        })
        .collect();

    for key in &issues_to_remove {
        // File format: {repo_key}-issue-{number}.md
        // Key format: {repo_key}-{number}
        // We need to transform key to filename
        if let Some(last_dash) = key.rfind('-') {
            let repo_key = &key[..last_dash];
            let number = &key[last_dash + 1..];
            let filename = format!("{repo_key}-issue-{number}.md");
            let file_path = contexts_dir.join(&filename);
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(&file_path) {
                    log::warn!("Failed to remove orphaned issue context {filename}: {e}");
                } else {
                    deleted_count += 1;
                }
            }
        }
        refs.issues.remove(key);
    }

    // Clean up orphaned PRs
    let prs_to_remove: Vec<String> = refs
        .prs
        .iter()
        .filter_map(|(key, entry)| {
            if let Some(orphaned_at) = entry.orphaned_at {
                if orphaned_at + retention_secs < now {
                    return Some(key.clone());
                }
            }
            None
        })
        .collect();

    for key in &prs_to_remove {
        // File format: {repo_key}-pr-{number}.md
        // Key format: {repo_key}-{number}
        if let Some(last_dash) = key.rfind('-') {
            let repo_key = &key[..last_dash];
            let number = &key[last_dash + 1..];
            let filename = format!("{repo_key}-pr-{number}.md");
            let file_path = contexts_dir.join(&filename);
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(&file_path) {
                    log::warn!("Failed to remove orphaned PR context {filename}: {e}");
                } else {
                    deleted_count += 1;
                }
            }
        }
        refs.prs.remove(key);
    }

    // Clean up orphaned security alerts
    let security_to_remove: Vec<String> = refs
        .security
        .iter()
        .filter_map(|(key, entry)| {
            if let Some(orphaned_at) = entry.orphaned_at {
                if orphaned_at + retention_secs < now {
                    return Some(key.clone());
                }
            }
            None
        })
        .collect();

    for key in &security_to_remove {
        // File format: {repo_key}-security-{number}.md
        // Key format: {repo_key}-{number}
        if let Some(last_dash) = key.rfind('-') {
            let repo_key = &key[..last_dash];
            let number = &key[last_dash + 1..];
            let filename = format!("{repo_key}-security-{number}.md");
            let file_path = contexts_dir.join(&filename);
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(&file_path) {
                    log::warn!("Failed to remove orphaned security context {filename}: {e}");
                } else {
                    deleted_count += 1;
                }
            }
        }
        refs.security.remove(key);
    }

    // Clean up orphaned advisories
    let advisories_to_remove: Vec<String> = refs
        .advisories
        .iter()
        .filter_map(|(key, entry)| {
            if let Some(orphaned_at) = entry.orphaned_at {
                if orphaned_at + retention_secs < now {
                    return Some(key.clone());
                }
            }
            None
        })
        .collect();

    for key in &advisories_to_remove {
        if let Some((owner, repo, ghsa_id)) = parse_advisory_context_key(key) {
            let adv_repo_key = format!("{owner}-{repo}");
            let filename = format!("{adv_repo_key}-advisory-{ghsa_id}.md");
            let file_path = contexts_dir.join(&filename);
            if file_path.exists() {
                if let Err(e) = std::fs::remove_file(&file_path) {
                    log::warn!("Failed to remove orphaned advisory context {filename}: {e}");
                } else {
                    deleted_count += 1;
                }
            }
        }
        refs.advisories.remove(key);
    }

    save_context_references(app, &refs)?;
    Ok(deleted_count)
}

/// Load/refresh issue context for a session by fetching data from GitHub
///
/// Context is stored in shared location: `git-context/{repo_key}-issue-{number}.md`
/// Multiple sessions can reference the same context file.
#[tauri::command]
pub async fn load_issue_context(
    app: tauri::AppHandle,
    session_id: String,
    issue_number: u32,
    project_path: String,
) -> Result<LoadedIssueContext, String> {
    crate::backend_runtime::context_service(&app)?
        .load_issue(
            &crate::backend_runtime::github_service(&app),
            &session_id,
            issue_number,
            &project_path,
        )
        .map_err(|error| error.to_string())
}

/// List all loaded issue contexts for a session
#[tauri::command]
pub async fn list_loaded_issue_contexts(
    app: tauri::AppHandle,
    session_id: String,
    worktree_id: Option<String>,
) -> Result<Vec<LoadedIssueContext>, String> {
    crate::backend_runtime::context_service(&app)?
        .list_issues(&session_id, worktree_id.as_deref())
        .map_err(|error| error.to_string())
}

/// Delete all context references for a session
///
/// Called during session deletion. Uses reference tracking - marks contexts as orphaned
/// but doesn't immediately delete shared files (they'll be cleaned up later by cleanup_orphaned_contexts).
pub fn cleanup_issue_contexts_for_session(
    app: &tauri::AppHandle,
    session_id: &str,
) -> Result<(), String> {
    log::trace!("Cleaning up contexts for session {session_id}");

    // Remove all references for this session (handles issues, PRs, security alerts, and advisories)
    let (orphaned_issues, orphaned_prs, orphaned_security, orphaned_advisories) =
        remove_all_session_references(app, session_id)?;

    log::trace!(
        "Marked {} issues, {} PRs, {} security alerts, and {} advisories as orphaned for session {session_id}",
        orphaned_issues.len(),
        orphaned_prs.len(),
        orphaned_security.len(),
        orphaned_advisories.len()
    );

    Ok(())
}

/// Remove a loaded issue context for a session
#[tauri::command]
pub async fn remove_issue_context(
    app: tauri::AppHandle,
    session_id: String,
    issue_number: u32,
    project_path: String,
) -> Result<(), String> {
    crate::backend_runtime::context_service(&app)?
        .remove_issue(&session_id, issue_number, &project_path)
        .map_err(|error| error.to_string())
}

// =============================================================================
// GitHub Pull Request Types and Commands
// =============================================================================

/// GitHub inline review comment (on specific diff lines), normalized to camelCase for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubReviewComment {
    pub author: GitHubAuthor,
    pub body: String,
    pub created_at: String,
    pub diff_hunk: String,
    pub path: String,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub line: Option<u32>,
}

/// Raw GraphQL response for PR review threads.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsResponse {
    data: ReviewThreadsData,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsData {
    repository: ReviewThreadsRepository,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsRepository {
    pull_request: ReviewThreadsPullRequest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadsPullRequest {
    review_threads: ReviewThreadConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadConnection {
    nodes: Vec<ReviewThread>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThread {
    is_outdated: bool,
    comments: ReviewThreadCommentConnection,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewThreadCommentConnection {
    nodes: Vec<RawGraphqlReviewComment>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGraphqlReviewComment {
    author: Option<RawGraphqlAuthor>,
    body: String,
    created_at: String,
    diff_hunk: String,
    path: String,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    line: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGraphqlAuthor {
    login: String,
}

impl From<RawGraphqlReviewComment> for GitHubReviewComment {
    fn from(raw: RawGraphqlReviewComment) -> Self {
        Self {
            author: GitHubAuthor {
                login: raw
                    .author
                    .map(|author| author.login)
                    .unwrap_or_else(|| "unknown".to_string()),
            },
            body: raw.body,
            created_at: raw.created_at,
            diff_hunk: raw.diff_hunk,
            path: raw.path,
            start_line: raw.start_line,
            line: raw.line,
        }
    }
}

fn current_review_comments_from_threads(threads: Vec<ReviewThread>) -> Vec<GitHubReviewComment> {
    threads
        .into_iter()
        .filter(|thread| !thread.is_outdated)
        .flat_map(|thread| thread.comments.nodes)
        .map(GitHubReviewComment::from)
        .collect()
}

/// List GitHub pull requests for a repository
///
/// Uses `gh pr list` to fetch PRs from the repository.
/// - state: "open", "closed", "merged", or "all" (default: "open")
/// - Returns up to 100 PRs sorted by creation date (newest first)
#[tauri::command]
pub async fn list_github_prs(
    app: AppHandle,
    project_path: String,
    state: Option<String>,
) -> Result<Vec<GitHubPullRequest>, String> {
    crate::backend_runtime::github_service(&app)
        .list_pull_requests(&project_path, state.as_deref())
        .map_err(|error| error.to_string())
}

/// Search GitHub pull requests using GitHub's search syntax
///
/// Uses `gh pr list --search` to query GitHub's search API.
/// This finds PRs beyond the default -L 100 limit.
#[tauri::command]
pub async fn search_github_prs(
    app: AppHandle,
    project_path: String,
    query: String,
) -> Result<Vec<GitHubPullRequest>, String> {
    crate::backend_runtime::github_service(&app)
        .search_pull_requests(&project_path, &query)
        .map_err(|error| error.to_string())
}

/// Get a GitHub PR by number, returning the same type as list_github_prs.
///
/// Uses `gh pr view` to fetch a single PR by exact number.
/// This finds any PR regardless of age or state.
#[tauri::command]
pub async fn get_github_pr_by_number(
    app: AppHandle,
    project_path: String,
    pr_number: u32,
) -> Result<GitHubPullRequest, String> {
    crate::backend_runtime::github_service(&app)
        .pull_request(&project_path, pr_number)
        .map_err(|error| error.to_string())
}

/// Get detailed information about a specific GitHub PR
///
/// Uses `gh pr view` to fetch the PR with comments and reviews.
#[tauri::command]
pub async fn get_github_pr(
    app: AppHandle,
    project_path: String,
    pr_number: u32,
) -> Result<GitHubPullRequestDetail, String> {
    crate::backend_runtime::github_service(&app)
        .pull_request_detail(&project_path, pr_number)
        .map_err(|error| error.to_string())
}

/// Fetch inline review comments for a PR.
///
/// Uses GitHub GraphQL review threads instead of REST review comments so GitHub
/// calculates `isOutdated` for us. REST exposes line/position fields, but those
/// are not a reliable way to infer whether GitHub considers a thread outdated.
#[tauri::command]
pub async fn get_pr_review_comments(
    app: AppHandle,
    project_path: String,
    pr_number: u32,
) -> Result<Vec<GitHubReviewComment>, String> {
    log::trace!("Getting review comments for PR #{pr_number} in {project_path}");

    let gh = resolve_gh_binary(&app);
    let repo_id = get_repo_identifier(&project_path)?;
    let query = r#"
query($owner: String!, $repo: String!, $prNumber: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $prNumber) {
      reviewThreads(first: 100) {
        nodes {
          isOutdated
          comments(first: 100) {
            nodes {
              author {
                login
              }
              body
              createdAt
              diffHunk
              path
              startLine
              line
            }
          }
        }
      }
    }
  }
}
"#;
    let args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("owner={}", repo_id.owner),
        "-f".to_string(),
        format!("repo={}", repo_id.repo),
        "-F".to_string(),
        format!("prNumber={pr_number}"),
        "-f".to_string(),
        format!("query={query}"),
    ];

    let output = gh_command(&gh, &project_path)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_gh_cli_auth_error(&stderr) {
            return Err("GitHub CLI not authenticated. Run 'gh auth login' first.".to_string());
        }
        if stderr.contains("404") || stderr.contains("Not Found") {
            return Err(format!("PR #{pr_number} not found"));
        }
        return Err(format!("gh api graphql failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: ReviewThreadsResponse =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse gh response: {e}"))?;

    let threads = response.data.repository.pull_request.review_threads.nodes;
    let total_threads = threads.len();
    let total_comments: usize = threads
        .iter()
        .map(|thread| thread.comments.nodes.len())
        .sum();
    let comments = current_review_comments_from_threads(threads);

    log::trace!(
        "Got {} current review comments for PR #{pr_number} ({} total comments across {} threads, outdated threads hidden)",
        comments.len(),
        total_comments,
        total_threads
    );
    Ok(comments)
}

/// Generate a branch name from a PR
/// e.g., PR #123 "Fix the login bug" -> "pr-123-fix-the-login-bug"
pub fn generate_branch_name_from_pr(pr_number: u32, title: &str) -> String {
    jean_core::generate_branch_name_from_pr(pr_number, title)
}

/// Format PR context as markdown for the context file
pub fn format_pr_context_markdown(ctx: &PullRequestContext) -> String {
    jean_core::format_pr_context_markdown(ctx)
}

/// Get the diff for a PR using `gh pr diff`
///
/// Returns the diff as a string, truncated to 100KB if too large.
pub fn get_pr_diff(
    project_path: &str,
    pr_number: u32,
    gh_binary: &std::path::Path,
) -> Result<String, String> {
    let gh = gh_binary.to_path_buf();
    let runner: jean_core::GhRunner = std::sync::Arc::new(move |path, args| {
        crate::platform::resolved_cli_command(&gh, Some(std::path::Path::new(path)))
            .args(args)
            .output()
            .map_err(|error| {
                jean_core::BackendError::new(
                    jean_core::BackendErrorCode::Io,
                    format!("Failed to run gh: {error}"),
                )
            })
    });
    jean_core::GitHubService::new(runner)
        .pull_request_diff(project_path, pr_number)
        .map_err(|error| error.to_string())
}

/// Load/refresh PR context for a session by fetching data from GitHub
///
/// Context is stored in shared location: `git-context/{repo_key}-pr-{number}.md`
/// Multiple sessions can reference the same context file.
#[tauri::command]
pub async fn load_pr_context(
    app: tauri::AppHandle,
    session_id: String,
    pr_number: u32,
    project_path: String,
) -> Result<LoadedPullRequestContext, String> {
    crate::backend_runtime::context_service(&app)?
        .load_pull_request(
            &crate::backend_runtime::github_service(&app),
            &session_id,
            pr_number,
            &project_path,
        )
        .map_err(|error| error.to_string())
}

/// List all loaded PR contexts for a session
#[tauri::command]
pub async fn list_loaded_pr_contexts(
    app: tauri::AppHandle,
    session_id: String,
    worktree_id: Option<String>,
) -> Result<Vec<LoadedPullRequestContext>, String> {
    crate::backend_runtime::context_service(&app)?
        .list_pull_requests(&session_id, worktree_id.as_deref())
        .map_err(|error| error.to_string())
}

/// Remove a loaded PR context for a session
#[tauri::command]
pub async fn remove_pr_context(
    app: tauri::AppHandle,
    session_id: String,
    pr_number: u32,
    project_path: String,
) -> Result<(), String> {
    crate::backend_runtime::context_service(&app)?
        .remove_pull_request(&session_id, pr_number, &project_path)
        .map_err(|error| error.to_string())
}

/// Get the content of a loaded issue context file
#[tauri::command]
pub async fn get_issue_context_content(
    app: tauri::AppHandle,
    session_id: String,
    issue_number: u32,
    project_path: String,
) -> Result<String, String> {
    crate::backend_runtime::context_service(&app)?
        .issue_content(&session_id, issue_number, &project_path)
        .map_err(|error| error.to_string())
}

/// Get the content of a loaded PR context file
#[tauri::command]
pub async fn get_pr_context_content(
    app: tauri::AppHandle,
    session_id: String,
    pr_number: u32,
    project_path: String,
) -> Result<String, String> {
    crate::backend_runtime::context_service(&app)?
        .pull_request_content(&session_id, pr_number, &project_path)
        .map_err(|error| error.to_string())
}

// =============================================================================
// Dependabot Alert / Security Types and Commands
// =============================================================================

/// Generate a branch name from a security alert
pub fn generate_branch_name_from_security_alert(
    alert_number: u32,
    package_name: &str,
    summary: &str,
) -> String {
    jean_core::generate_branch_name_from_security_alert(alert_number, package_name, summary)
}

/// Format security alert context as markdown
pub fn format_security_context_markdown(ctx: &SecurityAlertContext) -> String {
    jean_core::format_security_context_markdown(ctx)
}

/// Generate branch name from advisory
pub fn generate_branch_name_from_advisory(ghsa_id: &str, summary: &str) -> String {
    jean_core::generate_branch_name_from_advisory(ghsa_id, summary)
}

/// Format advisory context as markdown
pub fn format_advisory_context_markdown(ctx: &AdvisoryContext) -> String {
    jean_core::format_advisory_context_markdown(ctx)
}

/// List Dependabot alerts for a repository
///
/// Uses `gh api` to fetch Dependabot alerts from the repository.
/// - state: "open", "dismissed", "fixed", "auto_dismissed" (default: "open")
/// - Returns up to 100 alerts
#[tauri::command]
pub async fn list_dependabot_alerts(
    app: AppHandle,
    project_path: String,
    state: Option<String>,
) -> Result<Vec<DependabotAlert>, String> {
    let repository = crate::backend_runtime::git_service()
        .github_repository(&project_path)
        .map_err(|error| error.to_string())?;
    crate::backend_runtime::github_service(&app)
        .list_dependabot_alerts(&project_path, &repository, state.as_deref())
        .map_err(|error| error.to_string())
}

/// Get a single Dependabot alert by number
#[tauri::command]
pub async fn get_dependabot_alert(
    app: AppHandle,
    project_path: String,
    alert_number: u32,
) -> Result<DependabotAlert, String> {
    let repository = crate::backend_runtime::git_service()
        .github_repository(&project_path)
        .map_err(|error| error.to_string())?;
    crate::backend_runtime::github_service(&app)
        .dependabot_alert(&project_path, &repository, alert_number)
        .map_err(|error| error.to_string())
}

/// Load/refresh security alert context for a session by fetching data from GitHub
///
/// Context is stored in shared location: `git-context/{repo_key}-security-{number}.md`
/// Multiple sessions can reference the same context file.
#[tauri::command]
pub async fn load_security_alert_context(
    app: tauri::AppHandle,
    session_id: String,
    alert_number: u32,
    project_path: String,
) -> Result<LoadedSecurityAlertContext, String> {
    crate::backend_runtime::context_service(&app)?
        .load_security_alert(
            &crate::backend_runtime::github_service(&app),
            &session_id,
            alert_number,
            &project_path,
        )
        .map_err(|error| error.to_string())
}

/// List all loaded security alert contexts for a session
#[tauri::command]
pub async fn list_loaded_security_contexts(
    app: tauri::AppHandle,
    session_id: String,
    worktree_id: Option<String>,
) -> Result<Vec<LoadedSecurityAlertContext>, String> {
    crate::backend_runtime::context_service(&app)?
        .list_security_alerts(&session_id, worktree_id.as_deref())
        .map_err(|error| error.to_string())
}

/// Remove a loaded security alert context for a session
#[tauri::command]
pub async fn remove_security_context(
    app: tauri::AppHandle,
    session_id: String,
    alert_number: u32,
    project_path: String,
) -> Result<(), String> {
    crate::backend_runtime::context_service(&app)?
        .remove_security_alert(&session_id, alert_number, &project_path)
        .map_err(|error| error.to_string())
}

/// Get the content of a loaded security alert context file
#[tauri::command]
pub async fn get_security_context_content(
    app: tauri::AppHandle,
    session_id: String,
    alert_number: u32,
    project_path: String,
) -> Result<String, String> {
    crate::backend_runtime::context_service(&app)?
        .security_alert_content(&session_id, alert_number, &project_path)
        .map_err(|error| error.to_string())
}

// =============================================================================
// Repository Security Advisory Commands
// =============================================================================

/// List repository security advisories
///
/// Uses `gh api` to fetch security advisories from the repository.
/// - state: "draft", "published", "triage", "closed", or omit for all
/// - Returns up to 100 advisories
#[tauri::command]
pub async fn list_repository_advisories(
    app: AppHandle,
    project_path: String,
    state: Option<String>,
) -> Result<Vec<RepositoryAdvisory>, String> {
    let repository = crate::backend_runtime::git_service()
        .github_repository(&project_path)
        .map_err(|error| error.to_string())?;
    crate::backend_runtime::github_service(&app)
        .list_repository_advisories(&project_path, &repository, state.as_deref())
        .map_err(|error| error.to_string())
}

/// Get a single repository security advisory by GHSA ID
#[tauri::command]
pub async fn get_repository_advisory(
    app: AppHandle,
    project_path: String,
    ghsa_id: String,
) -> Result<RepositoryAdvisory, String> {
    let repository = crate::backend_runtime::git_service()
        .github_repository(&project_path)
        .map_err(|error| error.to_string())?;
    crate::backend_runtime::github_service(&app)
        .repository_advisory(&project_path, &repository, &ghsa_id)
        .map_err(|error| error.to_string())
}

/// Load/refresh advisory context for a session by fetching data from GitHub
///
/// Context is stored in shared location: `git-context/{repo_key}-advisory-{ghsa_id}.md`
/// Multiple sessions can reference the same context file.
#[tauri::command]
pub async fn load_advisory_context(
    app: tauri::AppHandle,
    session_id: String,
    ghsa_id: String,
    project_path: String,
) -> Result<LoadedAdvisoryContext, String> {
    crate::backend_runtime::context_service(&app)?
        .load_advisory(
            &crate::backend_runtime::github_service(&app),
            &session_id,
            &ghsa_id,
            &project_path,
        )
        .map_err(|error| error.to_string())
}

/// List all loaded advisory contexts for a session
#[tauri::command]
pub async fn list_loaded_advisory_contexts(
    app: tauri::AppHandle,
    session_id: String,
    worktree_id: Option<String>,
) -> Result<Vec<LoadedAdvisoryContext>, String> {
    crate::backend_runtime::context_service(&app)?
        .list_advisories(&session_id, worktree_id.as_deref())
        .map_err(|error| error.to_string())
}

/// Remove a loaded advisory context for a session
#[tauri::command]
pub async fn remove_advisory_context(
    app: tauri::AppHandle,
    session_id: String,
    ghsa_id: String,
    project_path: String,
) -> Result<(), String> {
    crate::backend_runtime::context_service(&app)?
        .remove_advisory(&session_id, &ghsa_id, &project_path)
        .map_err(|error| error.to_string())
}

/// Get the content of a loaded advisory context file
#[tauri::command]
pub async fn get_advisory_context_content(
    app: tauri::AppHandle,
    session_id: String,
    ghsa_id: String,
    project_path: String,
    worktree_id: Option<String>,
) -> Result<String, String> {
    crate::backend_runtime::context_service(&app)?
        .advisory_content(&session_id, worktree_id.as_deref(), &ghsa_id, &project_path)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphql_review_comment(body: &str) -> RawGraphqlReviewComment {
        RawGraphqlReviewComment {
            author: Some(RawGraphqlAuthor {
                login: "reviewer".to_string(),
            }),
            body: body.to_string(),
            created_at: "2026-05-11T12:00:00Z".to_string(),
            diff_hunk: "@@ -1 +1 @@".to_string(),
            path: "src/main.rs".to_string(),
            start_line: Some(10),
            line: Some(12),
        }
    }

    #[test]
    fn test_outdated_review_threads_are_filtered() {
        let comments = current_review_comments_from_threads(vec![
            ReviewThread {
                is_outdated: false,
                comments: ReviewThreadCommentConnection {
                    nodes: vec![graphql_review_comment("current")],
                },
            },
            ReviewThread {
                is_outdated: true,
                comments: ReviewThreadCommentConnection {
                    nodes: vec![graphql_review_comment("outdated")],
                },
            },
        ]);

        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].body, "current");
    }

    #[test]
    fn test_review_comment_conversion_preserves_fields() {
        let comment = GitHubReviewComment::from(graphql_review_comment("Please update this."));

        assert_eq!(comment.author.login, "reviewer");
        assert_eq!(comment.body, "Please update this.");
        assert_eq!(comment.created_at, "2026-05-11T12:00:00Z");
        assert_eq!(comment.diff_hunk, "@@ -1 +1 @@");
        assert_eq!(comment.path, "src/main.rs");
        assert_eq!(comment.start_line, Some(10));
        assert_eq!(comment.line, Some(12));
    }

    #[test]
    fn test_slugify_issue_title() {
        assert_eq!(
            slugify_issue_title("Fix the login bug"),
            "fix-the-login-bug"
        );
        // Apostrophe becomes space, so "can't" -> "can t" -> "can-t"
        assert_eq!(
            slugify_issue_title("Bug: can't save file"),
            "bug-can-t-save-file"
        );
        assert_eq!(slugify_issue_title("UPPERCASE Title"), "uppercase-title");
        assert_eq!(
            slugify_issue_title("Very long title that should be truncated to five words only"),
            "very-long-title-that-should"
        );
    }

    #[test]
    fn test_generate_branch_name_from_issue() {
        assert_eq!(
            generate_branch_name_from_issue(123, "Fix the login bug"),
            "issue-123-fix-the-login-bug"
        );
        assert_eq!(
            generate_branch_name_from_issue(42, "Add new feature"),
            "issue-42-add-new-feature"
        );
    }

    #[test]
    fn test_generate_branch_name_from_pr() {
        assert_eq!(
            generate_branch_name_from_pr(456, "Fix authentication"),
            "pr-456-fix-authentication"
        );
    }

    #[test]
    fn test_parse_context_key() {
        // Standard case: owner-repo-number
        assert_eq!(
            parse_context_key("owner-repo-123"),
            Some(("owner".to_string(), "repo".to_string(), 123))
        );

        // Repo with dash (splits on first dash for owner)
        assert_eq!(
            parse_context_key("owner-my-repo-456"),
            Some(("owner".to_string(), "my-repo".to_string(), 456))
        );

        // Invalid cases
        assert_eq!(parse_context_key("invalid"), None);
        assert_eq!(parse_context_key("repo-abc"), None);
        assert_eq!(parse_context_key("single"), None);
    }

    #[test]
    fn test_generate_branch_name_from_security_alert() {
        assert_eq!(
            generate_branch_name_from_security_alert(42, "lodash", "Prototype Pollution"),
            "security-42-lodash-prototype-pollution"
        );
        assert_eq!(
            generate_branch_name_from_security_alert(
                7,
                "@angular/core",
                "XSS vulnerability in template"
            ),
            "security-7-angular-core-xss-vulnerability-in-template"
        );
    }

    #[test]
    fn test_generate_branch_name_from_advisory() {
        let result = generate_branch_name_from_advisory(
            "GHSA-jg7v-5cqg-jvmf",
            "Prototype Pollution in lodash",
        );
        assert!(result.starts_with("advisory-jg7v-5cqg-jvmf-"));
        assert!(result.contains("prototype"));
    }

    #[test]
    fn test_gh_auth_error_excludes_unknown_github_host() {
        let stderr = "none of the git remotes configured for this repository point to a known GitHub host.\nTo tell gh about a new GitHub host, please use `gh auth login`";

        assert!(is_unsupported_github_repo_error(stderr));
        assert!(!is_gh_cli_auth_error(stderr));
    }

    #[test]
    fn test_gh_auth_error_excludes_missing_remotes() {
        let stderr = "no git remotes found";

        assert!(is_unsupported_github_repo_error(stderr));
        assert!(!is_gh_cli_auth_error(stderr));
    }

    #[test]
    fn test_gh_auth_error_detects_real_auth_prompt() {
        let stderr = "To get started with GitHub CLI, please run: gh auth login";

        assert!(!is_unsupported_github_repo_error(stderr));
        assert!(is_gh_cli_auth_error(stderr));
    }

    #[test]
    fn test_advisory_refs_match_session_or_worktree_ref() {
        let session_refs = vec!["owner-repo::GHSA-session-1111".to_string()];
        let worktree_refs = vec!["owner-repo::GHSA-worktree-2222".to_string()];

        assert!(advisory_refs_contain_expected_key(
            &session_refs,
            Some(&worktree_refs),
            "owner-repo::GHSA-session-1111"
        ));
        assert!(advisory_refs_contain_expected_key(
            &session_refs,
            Some(&worktree_refs),
            "owner-repo::GHSA-worktree-2222"
        ));
        assert!(!advisory_refs_contain_expected_key(
            &session_refs,
            Some(&worktree_refs),
            "owner-repo::GHSA-missing-3333"
        ));
    }

    #[test]
    fn test_parse_advisory_context_key() {
        assert_eq!(
            parse_advisory_context_key("owner-repo::GHSA-jg7v-5cqg-jvmf"),
            Some((
                "owner".to_string(),
                "repo".to_string(),
                "GHSA-jg7v-5cqg-jvmf".to_string()
            ))
        );

        // Invalid cases
        assert_eq!(parse_advisory_context_key("owner-repo-123"), None);
        assert_eq!(parse_advisory_context_key("invalid"), None);
    }
}
