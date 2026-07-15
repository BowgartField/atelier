use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

use super::github_issues::{
    get_github_contexts_dir, load_context_references, save_context_references, IssueContext,
};

pub use jean_core::{
    LinearIssue, LinearIssueContext, LinearIssueDetail, LinearIssueListResult, LinearIssueState,
    LinearTeam,
};

// =============================================================================
// Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedLinearIssueContext {
    pub identifier: String,
    pub title: String,
    pub comment_count: usize,
    pub project_name: String,
    pub url: Option<String>,
}

// =============================================================================
// GraphQL Client
// =============================================================================

const MAX_LINEAR_CONTEXT_IMAGE_BYTES: usize = 15 * 1024 * 1024;

// =============================================================================
// Helpers
// =============================================================================

/// Extract numeric part from Linear identifier (e.g., "ENG-123" → 123)
pub fn parse_linear_identifier_number(identifier: &str) -> u32 {
    identifier
        .rsplit_once('-')
        .and_then(|(_, num)| num.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Generate branch name from Linear issue identifier and title
pub fn generate_branch_name_from_linear_issue(identifier: &str, title: &str) -> String {
    jean_core::generate_branch_name_from_linear_issue(identifier, title)
}

/// Convert a LinearIssueDetail to the shared IssueContext for create_worktree
pub fn linear_issue_to_issue_context(detail: &LinearIssueDetail) -> IssueContext {
    use super::github_issues::GitHubComment;

    let comments = detail
        .comments
        .iter()
        .map(|c| GitHubComment {
            body: c.body.clone(),
            author: super::github_issues::GitHubAuthor {
                login: c
                    .user
                    .as_ref()
                    .map(|u| u.display_name.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
            },
            created_at: c.created_at.clone(),
        })
        .collect();

    IssueContext {
        number: parse_linear_identifier_number(&detail.identifier),
        title: detail.title.clone(),
        body: detail.description.clone(),
        comments,
    }
}

/// Format a Linear issue as markdown context
pub fn format_linear_issue_context_markdown(detail: &LinearIssueDetail) -> String {
    let mut content = String::new();

    content.push_str(&format!(
        "# Linear Issue {}: {}\n\n",
        detail.identifier, detail.title
    ));

    content.push_str(&format!("- **Status**: {}\n", detail.state.name));
    content.push_str(&format!("- **Priority**: {}\n", detail.priority_label));

    if !detail.labels.is_empty() {
        let labels: Vec<&str> = detail.labels.iter().map(|l| l.name.as_str()).collect();
        content.push_str(&format!("- **Labels**: {}\n", labels.join(", ")));
    }

    if let Some(assignee) = &detail.assignee {
        content.push_str(&format!("- **Assignee**: {}\n", assignee.display_name));
    }

    content.push_str(&format!("- **URL**: {}\n", detail.url));

    content.push_str("\n---\n\n");

    content.push_str("## Description\n\n");
    if let Some(desc) = &detail.description {
        if !desc.is_empty() {
            content.push_str(desc);
        } else {
            content.push_str("*No description provided.*");
        }
    } else {
        content.push_str("*No description provided.*");
    }
    content.push_str("\n\n");

    if !detail.comments.is_empty() {
        content.push_str("## Comments\n\n");
        for comment in &detail.comments {
            let author = comment
                .user
                .as_ref()
                .map(|u| u.display_name.as_str())
                .unwrap_or("Unknown");
            content.push_str(&format!("### {} ({})\n\n", author, comment.created_at));
            content.push_str(&comment.body);
            content.push_str("\n\n---\n\n");
        }
    }

    content.push_str("---\n\n");
    content.push_str("*Investigate this issue and propose a solution.*\n");

    content
}

fn is_trusted_linear_image_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    parsed
        .host_str()
        .map(|host| host == "uploads.linear.app" || host.ends_with(".uploads.linear.app"))
        .unwrap_or(false)
}

fn extract_linear_image_urls(markdown: &str) -> Vec<String> {
    let markdown_image_re = regex::Regex::new(r"!\[[^\]]*\]\(\s*<?([^)\s>]+)>?\s*\)").unwrap();
    let html_image_re =
        regex::Regex::new(r#"<img\b[^>]*\bsrc\s*=\s*["']([^"']+)["'][^>]*>"#).unwrap();
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for captures in markdown_image_re.captures_iter(markdown) {
        if let Some(url) = captures.get(1).map(|m| m.as_str()) {
            if is_trusted_linear_image_url(url) && seen.insert(url.to_string()) {
                urls.push(url.to_string());
            }
        }
    }

    for captures in html_image_re.captures_iter(markdown) {
        if let Some(url) = captures.get(1).map(|m| m.as_str()) {
            if is_trusted_linear_image_url(url) && seen.insert(url.to_string()) {
                urls.push(url.to_string());
            }
        }
    }

    urls
}

fn rewrite_linear_image_urls(markdown: &str, replacements: &HashMap<String, PathBuf>) -> String {
    let markdown_image_re = regex::Regex::new(r"!\[([^\]]*)\]\(\s*<?([^)\s>]+)>?\s*\)").unwrap();
    let html_image_re =
        regex::Regex::new(r#"(<img\b[^>]*\bsrc\s*=\s*["'])([^"']+)(["'][^>]*>)"#).unwrap();

    let rewritten = markdown_image_re.replace_all(markdown, |captures: &regex::Captures| {
        let alt = captures.get(1).map(|m| m.as_str()).unwrap_or("");
        let url = captures.get(2).map(|m| m.as_str()).unwrap_or("");
        match replacements.get(url) {
            Some(path) => format!("![{alt}](<{}>)", path.to_string_lossy()),
            None => captures
                .get(0)
                .map(|m| m.as_str())
                .unwrap_or("")
                .to_string(),
        }
    });

    html_image_re
        .replace_all(&rewritten, |captures: &regex::Captures| {
            let prefix = captures.get(1).map(|m| m.as_str()).unwrap_or("");
            let url = captures.get(2).map(|m| m.as_str()).unwrap_or("");
            let suffix = captures.get(3).map(|m| m.as_str()).unwrap_or("");
            match replacements.get(url) {
                Some(path) => format!("{prefix}{}{suffix}", path.to_string_lossy()),
                None => captures
                    .get(0)
                    .map(|m| m.as_str())
                    .unwrap_or("")
                    .to_string(),
            }
        })
        .to_string()
}

fn linear_image_extension(url: &str, content_type: Option<&str>) -> &'static str {
    match content_type
        .and_then(|ct| ct.split(';').next())
        .unwrap_or("")
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
) -> Result<PathBuf, String> {
    if !is_trusted_linear_image_url(url) {
        return Err(format!("Refusing untrusted Linear image URL: {url}"));
    }

    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    let mut response = client
        .get(url)
        .header("Authorization", api_key)
        .send()
        .await
        .map_err(|e| format!("Failed to download Linear image: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("Linear image download failed with status {status}"));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|s| s.to_string());
    if let Some(ct) = &content_type {
        if !ct.starts_with("image/") {
            return Err(format!(
                "Linear image URL returned non-image content type: {ct}"
            ));
        }
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed to read Linear image bytes: {e}"))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_LINEAR_CONTEXT_IMAGE_BYTES {
            return Err("Linear image is too large to cache".to_string());
        }
    }

    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create Linear image cache directory: {e}"))?;
    let extension = linear_image_extension(url, content_type.as_deref());
    let path = cache_dir.join(format!("{hash}.{extension}"));
    if !path.exists() {
        std::fs::write(&path, bytes)
            .map_err(|e| format!("Failed to write Linear image cache file: {e}"))?;
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
            Err(e) => {
                log::warn!("Failed to cache Linear context image: {e}");
            }
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

/// Linear config resolved from project + global preferences.
struct LinearConfig {
    api_key: String,
    project_name: String,
    team_id: Option<String>,
}

/// Get the Linear config for a project, falling back to global preferences for the API key.
fn get_linear_config(app: &AppHandle, project_id: &str) -> Result<LinearConfig, String> {
    let config = crate::backend_runtime::linear_service(app)?
        .config(project_id)
        .map_err(|error| error.to_string())?;
    Ok(LinearConfig {
        api_key: config.api_key,
        project_name: config.project_name,
        team_id: config.team_id,
    })
}

// =============================================================================
// Tauri Commands
// =============================================================================

/// List Linear teams for a project
#[tauri::command]
pub async fn list_linear_teams(
    app: AppHandle,
    project_id: String,
) -> Result<Vec<LinearTeam>, String> {
    crate::backend_runtime::linear_service(&app)?
        .list_teams(&project_id)
        .await
        .map_err(|error| error.to_string())
}

/// List Linear issues for a project (active states only)
#[tauri::command]
pub async fn list_linear_issues(
    app: AppHandle,
    project_id: String,
) -> Result<LinearIssueListResult, String> {
    crate::backend_runtime::linear_service(&app)?
        .list_issues(&project_id)
        .await
        .map_err(|error| error.to_string())
}

/// Search Linear issues
#[tauri::command]
pub async fn search_linear_issues(
    app: AppHandle,
    project_id: String,
    query: String,
) -> Result<Vec<LinearIssue>, String> {
    crate::backend_runtime::linear_service(&app)?
        .search_issues(&project_id, &query)
        .await
        .map_err(|error| error.to_string())
}

/// Get a single Linear issue with comments
#[tauri::command]
pub async fn get_linear_issue(
    app: AppHandle,
    project_id: String,
    issue_id: String,
) -> Result<LinearIssueDetail, String> {
    crate::backend_runtime::linear_service(&app)?
        .issue(&project_id, &issue_id)
        .await
        .map_err(|error| error.to_string())
}

/// Get a single Linear issue by its number (e.g., #12 → ENG-12)
#[tauri::command]
pub async fn get_linear_issue_by_number(
    app: AppHandle,
    project_id: String,
    issue_number: i64,
) -> Result<Option<LinearIssue>, String> {
    crate::backend_runtime::linear_service(&app)?
        .issue_by_number(&project_id, issue_number)
        .await
        .map_err(|error| error.to_string())
}

/// Load/refresh Linear issue context for a session
#[tauri::command]
pub async fn load_linear_issue_context(
    app: AppHandle,
    session_id: String,
    project_id: String,
    issue_id: String,
) -> Result<LoadedLinearIssueContext, String> {
    log::trace!("Loading Linear issue {issue_id} context for session {session_id}");

    let config = get_linear_config(&app, &project_id)?;
    let project_name = config.project_name;

    let mut detail = get_linear_issue(app.clone(), project_id, issue_id).await?;

    // Write to shared git-context directory
    let contexts_dir = get_github_contexts_dir(&app)?;
    std::fs::create_dir_all(&contexts_dir)
        .map_err(|e| format!("Failed to create git-context directory: {e}"))?;

    let identifier_lower = detail.identifier.to_lowercase();
    let image_cache_dir = contexts_dir
        .join("linear-context-images")
        .join(&project_name)
        .join(&identifier_lower);
    cache_linear_context_images(&mut detail, &config.api_key, &image_cache_dir).await;

    let context_file = contexts_dir.join(format!("{project_name}-linear-{identifier_lower}.md"));
    let context_content = format_linear_issue_context_markdown(&detail);

    std::fs::write(&context_file, context_content)
        .map_err(|e| format!("Failed to write Linear issue context file: {e}"))?;

    // Add reference tracking
    add_linear_reference(&app, &project_name, &detail.identifier, &session_id)?;

    let comment_count = detail.comments.len();
    log::trace!(
        "Linear issue context loaded for {} ({comment_count} comments)",
        detail.identifier
    );

    Ok(LoadedLinearIssueContext {
        identifier: detail.identifier,
        title: detail.title,
        comment_count,
        project_name,
        url: Some(detail.url.clone()),
    })
}

/// List all loaded Linear issue contexts for a session
#[tauri::command]
pub async fn list_loaded_linear_issue_contexts(
    app: AppHandle,
    session_id: String,
    worktree_id: Option<String>,
    project_id: String,
) -> Result<Vec<LoadedLinearIssueContext>, String> {
    log::trace!("Listing loaded Linear issue contexts for session {session_id}");

    let config = get_linear_config(&app, &project_id)?;
    let project_name = config.project_name;

    let mut keys = get_session_linear_refs(&app, &session_id)?;

    if let Some(ref wt_id) = worktree_id {
        if let Ok(wt_keys) = get_session_linear_refs(&app, wt_id) {
            for key in wt_keys {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
    }

    if keys.is_empty() {
        return Ok(vec![]);
    }

    let contexts_dir = get_github_contexts_dir(&app)?;
    let mut contexts = Vec::new();

    for key in keys {
        // Key format: "{project_name}-{identifier}"
        if let Some(identifier) = key.strip_prefix(&format!("{project_name}-")) {
            let context_file = contexts_dir.join(format!(
                "{project_name}-linear-{}.md",
                identifier.to_lowercase()
            ));

            if let Ok(content) = std::fs::read_to_string(&context_file) {
                let title = content
                    .lines()
                    .next()
                    .and_then(|line| {
                        line.strip_prefix("# Linear Issue ")
                            .and_then(|rest| rest.split_once(": "))
                            .map(|(_, title)| title.to_string())
                    })
                    .unwrap_or_else(|| format!("Issue {identifier}"));

                let comment_count = content
                    .split("## Comments")
                    .nth(1)
                    .map(|section| section.lines().filter(|l| l.starts_with("### ")).count())
                    .unwrap_or(0);

                let url = content
                    .lines()
                    .find(|l| l.starts_with("- **URL**: "))
                    .and_then(|l| l.strip_prefix("- **URL**: "))
                    .map(|s| s.to_string());

                contexts.push(LoadedLinearIssueContext {
                    identifier: identifier.to_string(),
                    title,
                    comment_count,
                    project_name: project_name.clone(),
                    url,
                });
            }
        }
    }

    Ok(contexts)
}

/// Content of a loaded Linear issue context file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueContextContent {
    pub identifier: String,
    pub title: String,
    pub content: String,
}

/// Return the full markdown content of each loaded Linear issue context file.
/// Used to embed context directly into investigation prompts (Claude CLI cannot access Linear API).
#[tauri::command]
pub async fn get_linear_issue_context_contents(
    app: AppHandle,
    session_id: String,
    worktree_id: Option<String>,
    project_id: String,
) -> Result<Vec<LinearIssueContextContent>, String> {
    log::trace!("Getting Linear issue context contents for session {session_id}");

    let config = get_linear_config(&app, &project_id)?;
    let project_name = config.project_name;

    let mut keys = get_session_linear_refs(&app, &session_id)?;

    if let Some(ref wt_id) = worktree_id {
        if let Ok(wt_keys) = get_session_linear_refs(&app, wt_id) {
            for key in wt_keys {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
    }

    if keys.is_empty() {
        return Ok(vec![]);
    }

    let contexts_dir = get_github_contexts_dir(&app)?;
    let mut contents = Vec::new();

    for key in keys {
        // Key format: "{project_name}-{identifier}"
        if let Some(identifier) = key.strip_prefix(&format!("{project_name}-")) {
            let context_file = contexts_dir.join(format!(
                "{project_name}-linear-{}.md",
                identifier.to_lowercase()
            ));

            if let Ok(content) = std::fs::read_to_string(&context_file) {
                let title = content
                    .lines()
                    .next()
                    .and_then(|line| {
                        line.strip_prefix("# Linear Issue ")
                            .and_then(|rest| rest.split_once(": "))
                            .map(|(_, title)| title.to_string())
                    })
                    .unwrap_or_else(|| format!("Issue {identifier}"));

                contents.push(LinearIssueContextContent {
                    identifier: identifier.to_string(),
                    title,
                    content,
                });
            }
        }
    }

    Ok(contents)
}

/// Remove a loaded Linear issue context from a session
#[tauri::command]
pub async fn remove_linear_issue_context(
    app: AppHandle,
    session_id: String,
    project_id: String,
    identifier: String,
) -> Result<(), String> {
    log::trace!("Removing Linear issue {identifier} context for session {session_id}");

    let config = get_linear_config(&app, &project_id)?;
    let project_name = config.project_name;

    let orphaned = remove_linear_reference(&app, &project_name, &identifier, &session_id)?;

    if orphaned {
        // Delete the context file if no more references
        let contexts_dir = get_github_contexts_dir(&app)?;
        let identifier_lower = identifier.to_lowercase();
        let context_file =
            contexts_dir.join(format!("{project_name}-linear-{identifier_lower}.md"));
        if context_file.exists() {
            let _ = std::fs::remove_file(&context_file);
        }
    }

    Ok(())
}

// =============================================================================
// Context Reference Tracking
// =============================================================================

/// Add a Linear issue reference for a session
/// Key format: "{project_name}-{identifier}"
pub fn add_linear_reference(
    app: &AppHandle,
    project_name: &str,
    identifier: &str,
    session_id: &str,
) -> Result<(), String> {
    let mut refs = load_context_references(app)?;
    let key = format!("{project_name}-{identifier}");

    let entry = refs.linear.entry(key).or_default();
    if !entry.sessions.contains(&session_id.to_string()) {
        entry.sessions.push(session_id.to_string());
    }
    entry.orphaned_at = None;

    save_context_references(app, &refs)
}

/// Remove a Linear issue reference for a session
pub fn remove_linear_reference(
    app: &AppHandle,
    project_name: &str,
    identifier: &str,
    session_id: &str,
) -> Result<bool, String> {
    let mut refs = load_context_references(app)?;
    let key = format!("{project_name}-{identifier}");

    let orphaned = if let Some(entry) = refs.linear.get_mut(&key) {
        entry.sessions.retain(|s| s != session_id);
        if entry.sessions.is_empty() && entry.orphaned_at.is_none() {
            entry.orphaned_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
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

/// Get all Linear issue keys referenced by a session
pub fn get_session_linear_refs(app: &AppHandle, session_id: &str) -> Result<Vec<String>, String> {
    let refs = load_context_references(app)?;
    Ok(refs
        .linear
        .iter()
        .filter(|(_, entry)| entry.sessions.contains(&session_id.to_string()))
        .map(|(key, _)| key.clone())
        .collect())
}

/// Get Linear issue identifiers (e.g. "ENG-123") referenced by a session, filtered by project name.
pub fn get_session_linear_identifiers(
    app: &AppHandle,
    session_id: &str,
    project_name: &str,
) -> Result<Vec<String>, String> {
    let keys = get_session_linear_refs(app, session_id)?;
    let prefix = format!("{project_name}-");
    Ok(keys
        .into_iter()
        .filter_map(|key| key.strip_prefix(&prefix).map(|id| id.to_string()))
        .collect())
}

/// Convert a LinearIssueContext into a LinearIssueDetail for formatting
pub fn linear_context_to_detail(ctx: &LinearIssueContext) -> LinearIssueDetail {
    LinearIssueDetail {
        id: ctx.id.clone(),
        identifier: ctx.identifier.clone(),
        title: ctx.title.clone(),
        description: ctx.description.clone(),
        state: LinearIssueState {
            name: "Unknown".to_string(),
            state_type: "started".to_string(),
            color: "#000000".to_string(),
        },
        labels: vec![],
        assignee: None,
        created_at: String::new(),
        url: String::new(),
        priority: 0,
        priority_label: "No priority".to_string(),
        comments: ctx.comments.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn rewrite_linear_image_urls_replaces_markdown_and_html_images() {
        let markdown = "Before ![shot](https://uploads.linear.app/a.png) after <img alt=\"diagram\" src=\"https://uploads.linear.app/b.jpg\" />";
        let mut replacements = HashMap::new();
        replacements.insert(
            "https://uploads.linear.app/a.png".to_string(),
            Path::new("/tmp/linear/a.png").to_path_buf(),
        );
        replacements.insert(
            "https://uploads.linear.app/b.jpg".to_string(),
            Path::new("/tmp/linear/b.jpg").to_path_buf(),
        );

        let rewritten = rewrite_linear_image_urls(markdown, &replacements);

        assert_eq!(
            rewritten,
            "Before ![shot](</tmp/linear/a.png>) after <img alt=\"diagram\" src=\"/tmp/linear/b.jpg\" />"
        );
    }

    #[test]
    fn extract_linear_image_urls_only_returns_trusted_linear_images() {
        let markdown = concat!(
            "![one](https://uploads.linear.app/a.png)\n",
            "<img src=\"https://uploads.linear.app/b.jpg\" />\n",
            "![skip](https://example.com/not-linear.png)\n",
            "![skip](https://workspace.linear.app/not-upload.png)\n",
            "![skip](http://uploads.linear.app/insecure.png)\n"
        );

        let urls = extract_linear_image_urls(markdown);

        assert_eq!(
            urls,
            vec![
                "https://uploads.linear.app/a.png".to_string(),
                "https://uploads.linear.app/b.jpg".to_string(),
            ]
        );
    }
}
