use crate::platform::silent_command;
use crate::platform::wsl_aware_command;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::types::{JeanConfig, MergeType};

fn gh_command(gh: &Path, repo_path: &str) -> std::process::Command {
    crate::platform::resolved_cli_command(gh, Some(Path::new(repo_path)))
}

/// Resolve the git metadata directories for a working directory.
///
/// Returns `(git_dir, git_common_dir)` as absolute, canonicalized paths.
/// - `git_dir`: worktree-specific metadata (e.g., `/repo/.git/worktrees/feat-x`)
/// - `git_common_dir`: shared metadata (e.g., `/repo/.git`)
///
/// For non-worktree repos, both point to the same `.git` directory.
/// Returns `None` if the path is not a git repo or the command fails.
pub fn resolve_git_dirs(working_dir: &Path) -> Option<(String, String)> {
    let output = silent_command("git")
        .args(["rev-parse", "--git-dir", "--git-common-dir"])
        .current_dir(working_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let git_dir_raw = lines.next()?.trim();
    let git_common_dir_raw = lines.next()?.trim();

    let resolve = |p: &str| -> Option<String> {
        let path = Path::new(p);
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            working_dir.join(path)
        };
        std::fs::canonicalize(&abs)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    };

    Some((resolve(git_dir_raw)?, resolve(git_common_dir_raw)?))
}

/// Repository identifier extracted from GitHub remote URL
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoIdentifier {
    pub owner: String,
    pub repo: String,
}

impl RepoIdentifier {
    /// Create a key string for use in file naming: "{owner}-{repo}"
    pub fn to_key(&self) -> String {
        format!("{}-{}", self.owner, self.repo)
    }
}

/// Extract repository owner and name from a git repository's GitHub remote
///
/// Returns an error if:
/// - The repository has no origin remote
/// - The remote URL is not a GitHub URL
pub fn get_repo_identifier(repo_path: &str) -> Result<RepoIdentifier, String> {
    let github_url = get_github_url(repo_path)?;

    // Parse owner/repo from URL: https://github.com/owner/repo
    let url_without_prefix = github_url
        .strip_prefix("https://github.com/")
        .ok_or_else(|| format!("Invalid GitHub URL format: {github_url}"))?;

    let parts: Vec<&str> = url_without_prefix.split('/').collect();
    if parts.len() < 2 {
        return Err(format!(
            "Could not parse owner/repo from GitHub URL: {github_url}"
        ));
    }

    Ok(RepoIdentifier {
        owner: parts[0].to_string(),
        repo: parts[1].to_string(),
    })
}

/// Check if a path is a valid git repository
pub fn validate_git_repo(path: &str) -> Result<bool, String> {
    let path = Path::new(path);

    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }

    if !path.is_dir() {
        return Err(format!("Path is not a directory: {}", path.display()));
    }

    // Check for .git directory or file (could be a worktree)
    let git_path = path.join(".git");
    Ok(git_path.exists())
}

/// Initialize a new git repository at the given path
///
/// Creates the directory if it doesn't exist, runs `git init`, and creates an initial commit
pub fn init_repo(path: &str) -> Result<(), String> {
    let path_obj = Path::new(path);

    // Create directory if it doesn't exist
    if !path_obj.exists() {
        std::fs::create_dir_all(path_obj)
            .map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    // Check if directory already has .git
    let git_path = path_obj.join(".git");
    if git_path.exists() {
        // Check if it has any commits
        let has_commits = wsl_aware_command("git", Some(Path::new(path)))
            .args(["rev-parse", "HEAD"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if has_commits {
            return Err("Directory is already a git repository".to_string());
        }
        // No commits yet, skip git init and just create the initial commit
        log::trace!("Git repo exists but has no commits, will create initial commit");
    } else {
        // Run git init
        let output = wsl_aware_command("git", Some(Path::new(path)))
            .args(["init"])
            .output()
            .map_err(|e| format!("Failed to run git init: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git init failed: {stderr}"));
        }
    }

    // Create .gitkeep file so we have something to commit
    let gitkeep_path = path_obj.join(".gitkeep");
    std::fs::write(&gitkeep_path, "").map_err(|e| format!("Failed to create .gitkeep: {e}"))?;

    // Stage the file
    let add_output = wsl_aware_command("git", Some(Path::new(path)))
        .args(["add", ".gitkeep"])
        .output()
        .map_err(|e| format!("Failed to run git add: {e}"))?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(format!("git add failed: {stderr}"));
    }

    // Create initial commit
    let commit_output = wsl_aware_command("git", Some(Path::new(path)))
        .args(["commit", "-m", "jean's init vibe commit"])
        .output()
        .map_err(|e| format!("Failed to run git commit: {e}"))?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        return Err(format!("git commit failed: {stderr}"));
    }

    log::trace!("Successfully initialized git repository at {path} with initial commit");
    Ok(())
}

/// Extract a repository name from a git URL
///
/// Handles HTTPS (`https://github.com/user/repo.git`) and SSH (`git@github.com:user/repo.git`) formats.
/// Strips `.git` suffix if present.
pub fn extract_repo_name_from_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim().trim_end_matches('/');

    // Get the last path segment
    let name = trimmed
        .rsplit('/')
        .next()
        // SSH format: git@host:user/repo.git — split on ':'  then take last
        .or_else(|| trimmed.rsplit(':').next())
        .ok_or_else(|| format!("Could not extract repository name from URL: {url}"))?;

    // Strip .git suffix
    let name = name.strip_suffix(".git").unwrap_or(name);

    if name.is_empty() {
        return Err(format!("Could not extract repository name from URL: {url}"));
    }

    Ok(name.to_string())
}

/// Clone a git repository from a remote URL to a local destination
pub fn clone_repo(url: &str, destination: &str) -> Result<(), String> {
    let url = url.trim();

    // Basic URL format validation
    let valid_prefix = url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("git@")
        || url.starts_with("ssh://");

    if !valid_prefix {
        return Err(
            "Invalid git URL. Use HTTPS (https://...) or SSH (git@...) format.".to_string(),
        );
    }

    // Check destination doesn't already exist
    let dest_path = Path::new(destination);
    if dest_path.exists() {
        return Err(format!("Destination already exists: {destination}"));
    }

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
        }
    }

    log::info!("Cloning {url} into {destination}");

    let output = wsl_aware_command("git", None)
        .args(["clone", url, destination])
        .output()
        .map_err(|e| format!("Failed to run git clone: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git clone failed: {stderr}"));
    }

    log::info!("Successfully cloned repository to {destination}");
    Ok(())
}

/// Get the repository name from a path (last component of the path)
pub fn get_repo_name(path: &str) -> Result<String, String> {
    let path = Path::new(path);

    path.file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "Could not extract repository name from path: {}",
                path.display()
            )
        })
}

/// A GitHub remote with its name and resolved HTTPS URL
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRemote {
    pub name: String,
    pub url: String,
}

/// Convert a raw git remote URL to a GitHub HTTPS URL, if possible
fn normalize_github_url(remote_url: &str) -> Option<String> {
    if remote_url.starts_with("git@github.com:") {
        Some(
            remote_url
                .replace("git@github.com:", "https://github.com/")
                .trim_end_matches(".git")
                .to_string(),
        )
    } else if remote_url.starts_with("https://github.com/") {
        Some(remote_url.trim_end_matches(".git").to_string())
    } else {
        None
    }
}

/// Get the GitHub URL for a specific remote
pub fn get_github_url_for_remote(repo_path: &str, remote: &str) -> Result<String, String> {
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["remote", "get-url", remote])
        .output()
        .map_err(|e| format!("Failed to get remote URL: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to get remote URL: {stderr}"));
    }

    let remote_url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    normalize_github_url(&remote_url)
        .ok_or_else(|| format!("Remote URL is not a GitHub repository: {remote_url}"))
}

/// Get the GitHub URL for a repository (uses "origin" remote)
pub fn get_github_url(repo_path: &str) -> Result<String, String> {
    get_github_url_for_remote(repo_path, "origin")
}

/// Get all GitHub remotes for a repository
pub fn get_github_remotes(repo_path: &str) -> Result<Vec<GitHubRemote>, String> {
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["remote"])
        .output()
        .map_err(|e| format!("Failed to list remotes: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to list remotes: {stderr}"));
    }

    let remote_names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let mut result = Vec::new();

    for name in remote_names {
        if let Ok(url_out) = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["remote", "get-url", &name])
            .output()
        {
            if url_out.status.success() {
                let raw = String::from_utf8_lossy(&url_out.stdout).trim().to_string();
                if let Some(url) = normalize_github_url(&raw) {
                    result.push(GitHubRemote { name, url });
                }
            }
        }
    }

    result.sort_by_key(|r| if r.name == "origin" { 0 } else { 1 });

    Ok(result)
}

/// Get the current branch name (HEAD) for a repository
/// Uses symbolic-ref first (works on repos with no commits), falls back to rev-parse
pub fn get_current_branch(repo_path: &str) -> Result<String, String> {
    // Try symbolic-ref first — works even on empty repos (no commits yet)
    let sym_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["symbolic-ref", "--short", "HEAD"])
        .output();

    if let Ok(ref o) = sym_output {
        if o.status.success() {
            let branch = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !branch.is_empty() {
                return Ok(branch);
            }
        }
    }

    // Fall back to rev-parse (works when HEAD is detached)
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|e| format!("Failed to run git command: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to get current branch: {stderr}"));
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(branch)
}

/// Check if a branch exists in a repository
pub fn branch_exists(repo_path: &str, branch_name: &str) -> bool {
    crate::backend_runtime::git_service().branch_exists(repo_path, branch_name)
}

/// Check if a remote-tracking branch exists (refs/remotes/origin/<branch>)
pub fn remote_branch_exists(repo_path: &str, branch_name: &str) -> bool {
    crate::backend_runtime::git_service().remote_branch_exists(repo_path, branch_name)
}

/// Check if a repository has any commits
pub fn has_commits(repo_path: &str) -> bool {
    crate::backend_runtime::git_service().has_commits(repo_path)
}

/// Get a valid base branch for creating worktrees
///
/// Tries the provided branch first, then falls back to common defaults (main, master)
/// or the current branch if none of those exist.
/// Returns an error if the repository has no commits yet.
pub fn get_valid_base_branch(repo_path: &str, preferred_branch: &str) -> Result<String, String> {
    crate::backend_runtime::git_service()
        .valid_base_branch(repo_path, preferred_branch)
        .map_err(|error| error.to_string())
}

/// Rename the current branch to a new name
///
/// # Arguments
/// * `repo_path` - Path to the repository (can be a worktree)
/// * `new_name` - The new name for the branch
///
/// Returns the old branch name on success
pub fn rename_branch(repo_path: &str, new_name: &str) -> Result<String, String> {
    log::trace!("Renaming current branch to {new_name} in {repo_path}");

    // First check if we're in detached HEAD state
    let head_check = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .map_err(|e| format!("Failed to check HEAD state: {e}"))?;

    if !head_check.status.success() {
        return Err("Cannot rename branch: HEAD is detached".to_string());
    }

    let old_name = String::from_utf8_lossy(&head_check.stdout)
        .trim()
        .to_string();

    // Skip if already named the same
    if old_name == new_name {
        log::trace!("Branch already named {new_name}, skipping rename");
        return Ok(old_name);
    }

    // Check if target branch name already exists
    let branch_exists = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["rev-parse", "--verify", &format!("refs/heads/{}", new_name)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let final_name = if branch_exists {
        // Append suffix to make unique
        find_unique_branch_name(repo_path, new_name)?
    } else {
        new_name.to_string()
    };

    // Perform the rename
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["branch", "-m", &final_name])
        .output()
        .map_err(|e| format!("Failed to rename branch: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git branch rename failed: {stderr}"));
    }

    log::trace!("Successfully renamed branch from {old_name} to {final_name}");
    Ok(final_name)
}

/// Find a unique branch name by appending a random 4-char suffix
fn find_unique_branch_name(repo_path: &str, base_name: &str) -> Result<String, String> {
    use rand::Rng;

    for _ in 0..10 {
        let suffix: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(4)
            .map(char::from)
            .map(|c| c.to_ascii_lowercase())
            .collect();

        let candidate = format!("{base_name}-{suffix}");
        let exists = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["rev-parse", "--verify", &format!("refs/heads/{candidate}")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !exists {
            log::trace!("Using unique branch name: {candidate}");
            return Ok(candidate);
        }
    }
    Err("Could not find unique branch name after 10 attempts".to_string())
}

/// Fetch a branch from remote without merging (safe, no conflict risk)
pub fn git_fetch(repo_path: &str, branch: &str, remote: Option<&str>) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .fetch(repo_path, branch, remote)
        .map_err(|error| error.to_string())
}

/// Result of a PR-aware push operation
pub struct PushResult {
    pub output: String,
    /// true when we tried to push to the PR branch but failed and fell back to a regular push
    pub fell_back: bool,
    /// true when push failed due to permission/authentication errors (e.g. fork PR without write access)
    pub permission_denied: bool,
    /// Remote actually pushed to (e.g., "origin" or a fork owner). None on fallback/denied.
    pub pushed_remote: Option<String>,
    /// Branch on pushed_remote that was updated. None on fallback/denied.
    pub pushed_branch: Option<String>,
}

/// Check if a git push stderr indicates a permission/authentication error
fn is_permission_error(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("403")
        || lower.contains("could not read username")
        || lower.contains("write access")
        || lower.contains("authentication failed")
        || lower.contains("not allowed")
}

/// Push to a PR's remote branch, handling fork PRs by adding the fork remote if needed.
/// Uses --force-with-lease for safety. Falls back to regular push (new branch) on failure.
///
/// Flow:
/// 1. Query gh pr view for fork info
/// 2. Same-repo PR: push to origin
/// 3. Fork PR: add fork remote if needed, fetch, push
/// 4. On failure: fall back to regular push (git push -u origin HEAD)
pub fn git_push_to_pr(
    repo_path: &str,
    pr_number: u32,
    gh_binary: &std::path::Path,
) -> Result<PushResult, String> {
    log::trace!("Pushing to PR #{pr_number} remote branch in {repo_path}");

    // 1. Query PR info from GitHub
    let gh_output = gh_command(gh_binary, repo_path)
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "headRefName,isCrossRepository,headRepositoryOwner,headRepository",
        ])
        .output()
        .map_err(|e| format!("Failed to run gh pr view: {e}"))?;

    if !gh_output.status.success() {
        let stderr = String::from_utf8_lossy(&gh_output.stderr).to_string();
        log::warn!("gh pr view failed, falling back to regular push: {stderr}");
        let output = crate::backend_runtime::git_service()
            .push(repo_path, None)
            .map_err(|error| error.to_string())?
            .output;
        return Ok(PushResult {
            output,
            fell_back: true,
            permission_denied: false,
            pushed_remote: None,
            pushed_branch: None,
        });
    }

    let pr_info: serde_json::Value = serde_json::from_slice(&gh_output.stdout)
        .map_err(|e| format!("Failed to parse gh pr view output: {e}"))?;

    let head_ref_name = pr_info["headRefName"]
        .as_str()
        .ok_or_else(|| "Missing headRefName in PR info".to_string())?;
    let is_cross_repository = pr_info["isCrossRepository"].as_bool().unwrap_or(false);

    if !is_cross_repository {
        // Same-repo PR: push HEAD to origin/{head_ref_name} with --force-with-lease
        let refspec = format!("HEAD:{head_ref_name}");
        log::trace!("Same-repo PR, pushing {refspec} to origin");
        let output = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["push", "--force-with-lease", "origin", &refspec])
            .output()
            .map_err(|e| format!("Failed to run git push: {e}"))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let result = if stdout.is_empty() { stderr } else { stdout };
            log::trace!("Successfully pushed to origin/{head_ref_name}");
            return Ok(PushResult {
                output: result,
                fell_back: false,
                permission_denied: false,
                pushed_remote: Some("origin".to_string()),
                pushed_branch: Some(head_ref_name.to_string()),
            });
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            if is_permission_error(&stderr) {
                log::warn!("Permission denied pushing to origin/{head_ref_name}: {stderr}");
                return Ok(PushResult {
                    output: stderr,
                    fell_back: false,
                    permission_denied: true,
                    pushed_remote: None,
                    pushed_branch: None,
                });
            }
            log::warn!(
                "Failed to push to origin/{head_ref_name}, falling back to regular push: {stderr}"
            );
            let fallback_output = crate::backend_runtime::git_service()
                .push(repo_path, None)
                .map_err(|error| error.to_string())?
                .output;
            return Ok(PushResult {
                output: fallback_output,
                fell_back: true,
                permission_denied: false,
                pushed_remote: None,
                pushed_branch: None,
            });
        }
    }

    // Fork PR: need to add fork remote and push there
    let fork_owner = pr_info["headRepositoryOwner"]["login"]
        .as_str()
        .ok_or_else(|| "Missing headRepositoryOwner.login in PR info".to_string())?;
    let fork_repo_name = pr_info["headRepository"]["name"]
        .as_str()
        .ok_or_else(|| "Missing headRepository.name in PR info".to_string())?;

    log::trace!("Fork PR from {fork_owner}/{fork_repo_name}, branch {head_ref_name}");

    // Determine URL scheme from origin
    let origin_url_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|e| format!("Failed to get origin URL: {e}"))?;

    let origin_url = String::from_utf8_lossy(&origin_url_output.stdout)
        .trim()
        .to_string();
    let fork_url = if origin_url.starts_with("git@") || origin_url.starts_with("ssh://") {
        format!("git@github.com:{fork_owner}/{fork_repo_name}.git")
    } else {
        format!("https://github.com/{fork_owner}/{fork_repo_name}.git")
    };

    log::trace!("Fork URL: {fork_url}");

    // Check if a remote for this fork already exists
    let remotes_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["remote", "-v"])
        .output()
        .map_err(|e| format!("Failed to list remotes: {e}"))?;

    let remotes_str = String::from_utf8_lossy(&remotes_output.stdout);
    let remote_name = remotes_str
        .lines()
        .find(|line| {
            line.contains(&fork_url) || line.contains(&format!("{fork_owner}/{fork_repo_name}"))
        })
        .and_then(|line| line.split_whitespace().next())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Add the fork remote
            log::trace!("Adding fork remote: {fork_owner} -> {fork_url}");
            let add_output = wsl_aware_command("git", Some(Path::new(repo_path)))
                .args(["remote", "add", fork_owner, &fork_url])
                .output();

            if let Err(e) = &add_output {
                log::warn!("Failed to add fork remote: {e}");
            } else if let Ok(out) = &add_output {
                if !out.status.success() {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    log::warn!("git remote add failed: {stderr}");
                }
            }

            fork_owner.to_string()
        });

    // Fetch the branch from the fork remote
    log::trace!("Fetching {head_ref_name} from {remote_name}");
    let fetch_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["fetch", &remote_name, head_ref_name])
        .output()
        .map_err(|e| format!("Failed to fetch from fork: {e}"))?;

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr).to_string();
        log::warn!("Fetch from fork failed (continuing with push): {stderr}");
    }

    // Push HEAD to the fork remote with --force-with-lease
    let refspec = format!("HEAD:{head_ref_name}");
    log::trace!("Pushing {refspec} to {remote_name} --force-with-lease");
    let push_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["push", "--force-with-lease", &remote_name, &refspec])
        .output()
        .map_err(|e| format!("Failed to push to fork: {e}"))?;

    if push_output.status.success() {
        let stdout = String::from_utf8_lossy(&push_output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&push_output.stderr).to_string();
        let result = if stdout.is_empty() { stderr } else { stdout };
        log::trace!("Successfully pushed to {remote_name}/{head_ref_name}");
        Ok(PushResult {
            output: result,
            fell_back: false,
            permission_denied: false,
            pushed_remote: Some(remote_name.clone()),
            pushed_branch: Some(head_ref_name.to_string()),
        })
    } else {
        let stderr = String::from_utf8_lossy(&push_output.stderr).to_string();
        if is_permission_error(&stderr) {
            log::warn!("Permission denied pushing to {remote_name}/{head_ref_name}: {stderr}");
            return Ok(PushResult {
                output: stderr,
                fell_back: false,
                permission_denied: true,
                pushed_remote: None,
                pushed_branch: None,
            });
        }
        log::warn!("Failed to push to {remote_name}/{head_ref_name}, falling back to regular push: {stderr}");
        let fallback_output = crate::backend_runtime::git_service()
            .push(repo_path, None)
            .map_err(|error| error.to_string())?
            .output;
        Ok(PushResult {
            output: fallback_output,
            fell_back: true,
            permission_denied: false,
            pushed_remote: None,
            pushed_branch: None,
        })
    }
}

/// Set upstream tracking for a local branch to a remote branch.
/// Uses git config directly (more robust than --set-upstream-to when the
/// remote-tracking ref may not exist after fetch_pr_to_branch).
pub fn set_upstream_tracking(
    repo_path: &str,
    local_branch: &str,
    remote_branch: &str,
) -> Result<(), String> {
    log::trace!("Setting upstream for {local_branch} to origin/{remote_branch} in {repo_path}");

    let _ = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["config", &format!("branch.{local_branch}.remote"), "origin"])
        .output();

    let merge_ref = format!("refs/heads/{remote_branch}");
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args([
            "config",
            &format!("branch.{local_branch}.merge"),
            &merge_ref,
        ])
        .output()
        .map_err(|e| format!("Failed to set upstream config: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("Failed to set upstream config for {local_branch}: {stderr}");
    }
    Ok(())
}

/// Fetch from remote origin (best effort, ignores errors if no remote)
pub fn fetch_origin(repo_path: &str) -> Result<(), String> {
    log::trace!("Fetching from origin in {repo_path}");

    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["fetch", "origin"])
        .output()
        .map_err(|e| format!("Failed to run git fetch: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Don't fail if no remote - just log and continue
        if stderr.contains("does not appear to be a git repository")
            || stderr.contains("Could not read from remote")
            || stderr.contains("'origin' does not appear to be a git repository")
        {
            log::trace!("No remote origin available: {stderr}");
            return Ok(());
        }
        log::warn!("Failed to fetch from origin: {stderr}");
    } else {
        log::trace!("Successfully fetched from origin");
    }

    Ok(())
}

/// Get list of remote branches for a repository (strips origin/ prefix)
pub fn get_remote_branches(repo_path: &str) -> Result<Vec<String>, String> {
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["branch", "-r", "--format=%(refname:short)"])
        .output()
        .map_err(|e| format!("Failed to run git command: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to list remote branches: {stderr}"));
    }

    let branches: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        // Strip "origin/" prefix and filter out HEAD and bare "origin"
        .filter_map(|s| {
            // Skip HEAD references and bare remote name (origin/HEAD -> origin)
            if s.contains("HEAD") || s == "origin" {
                None
            } else if let Some(stripped) = s.strip_prefix("origin/") {
                Some(stripped.to_string())
            } else {
                // For other remotes, keep as-is but could strip their prefix too
                Some(s)
            }
        })
        .collect();

    Ok(branches)
}

/// Create a new git worktree
///
/// # Arguments
/// * `repo_path` - Path to the main repository
/// * `worktree_path` - Path where the worktree will be created
/// * `new_branch_name` - Name for the new branch to create
/// * `base_branch` - Branch to base the new branch on (e.g., "main")
pub fn create_worktree(
    repo_path: &str,
    worktree_path: &str,
    new_branch_name: &str,
    base_branch: &str,
) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .create_worktree(repo_path, worktree_path, new_branch_name, base_branch)
        .map_err(|error| error.to_string())
}

/// Create a worktree using an existing branch (no new branch created)
///
/// # Arguments
/// * `repo_path` - Path to the main repository
/// * `worktree_path` - Path where the worktree will be created
/// * `existing_branch` - Name of the existing branch to checkout
pub fn create_worktree_from_existing_branch(
    repo_path: &str,
    worktree_path: &str,
    existing_branch: &str,
) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .create_worktree_from_existing_branch(repo_path, worktree_path, existing_branch)
        .map_err(|error| error.to_string())
}

pub fn is_retryable_worktree_create_error(error: &str) -> bool {
    jean_core::git::retryable_worktree_create_error(error)
}

/// Checkout a PR using gh CLI in the specified directory
///
/// Uses `gh pr checkout <number>` which properly handles:
/// - Fetching the PR branch from forks
/// - Setting up proper tracking
/// - Checking out the actual PR branch
///
/// Fetch a PR ref into a local branch name, bypassing gh cli.
/// Used when the PR's head branch name collides with a locally checked-out branch.
pub fn fetch_pr_to_branch(
    repo_path: &str,
    pr_number: u32,
    local_branch: &str,
) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .fetch_pr_to_branch(repo_path, pr_number, local_branch)
        .map_err(|error| error.to_string())
}

/// Checkout an existing branch in a worktree
pub fn checkout_branch(worktree_path: &str, branch: &str) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .checkout_branch(worktree_path, branch)
        .map_err(|error| error.to_string())
}

/// # Arguments
/// * `worktree_path` - Path to the worktree where to checkout the PR
/// * `pr_number` - The PR number to checkout
/// * `branch_name` - Optional local branch name to use (ensures local matches remote)
pub fn gh_pr_checkout(
    worktree_path: &str,
    pr_number: u32,
    branch_name: Option<&str>,
    gh_binary: &std::path::Path,
) -> Result<String, String> {
    log::trace!("Running gh pr checkout {pr_number} in {worktree_path}");

    let pr_num_str = pr_number.to_string();
    let mut args = vec!["pr", "checkout", &pr_num_str];
    if let Some(name) = branch_name {
        args.extend(["-b", name]);
    }

    let output = gh_command(gh_binary, worktree_path)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run gh pr checkout: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to checkout PR #{pr_number}: {stderr}"));
    }

    // Get the current branch name after checkout
    let branch_output = wsl_aware_command("git", Some(Path::new(worktree_path)))
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .map_err(|e| format!("Failed to get branch name: {e}"))?;

    let branch_name = String::from_utf8_lossy(&branch_output.stdout)
        .trim()
        .to_string();

    log::trace!("Successfully checked out PR #{pr_number} to branch {branch_name}");
    Ok(branch_name)
}

/// Remove a git worktree
///
/// # Arguments
/// * `repo_path` - Path to the main repository
/// * `worktree_path` - Path to the worktree to remove
pub fn remove_worktree(repo_path: &str, worktree_path: &str) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .remove_worktree(repo_path, worktree_path)
        .map_err(|error| error.to_string())
}

/// Delete the branch associated with a worktree
///
/// # Arguments
/// * `repo_path` - Path to the main repository
/// * `branch_name` - Name of the branch to delete
pub fn delete_branch(repo_path: &str, branch_name: &str) -> Result<(), String> {
    crate::backend_runtime::git_service()
        .delete_branch(repo_path, branch_name)
        .map_err(|error| error.to_string())
}

/// Find which worktree (if any) has a given branch checked out.
/// Parses `git worktree list --porcelain` output. Returns the worktree path or None.
pub fn find_worktree_for_branch(repo_path: &str, branch: &str) -> Option<String> {
    crate::backend_runtime::git_service().find_worktree_for_branch(repo_path, branch)
}

/// Clean up a stale branch that may be checked out in a defunct worktree.
/// Used before PR checkout to handle archived/deleted worktrees whose branch still exists.
pub fn cleanup_stale_branch(repo_path: &str, branch: &str) {
    crate::backend_runtime::git_service().cleanup_stale_branch(repo_path, branch);
}

/// List existing worktrees for a repository
#[allow(dead_code)]
pub fn list_worktrees(repo_path: &str) -> Result<Vec<String>, String> {
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| format!("Failed to run git worktree list: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to list worktrees: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let worktrees = stdout
        .lines()
        .filter(|line| line.starts_with("worktree "))
        .map(|line| line.strip_prefix("worktree ").unwrap_or("").to_string())
        .collect();

    Ok(worktrees)
}

/// Open a pull request using the GitHub CLI (gh)
///
/// * `repo_path` - Path to the repository
/// * `title` - Optional PR title (if None, gh will prompt or use default)
/// * `body` - Optional PR body
/// * `draft` - Whether to create as draft PR
///
/// Returns the PR URL on success
pub fn open_pull_request(
    repo_path: &str,
    title: Option<&str>,
    body: Option<&str>,
    draft: bool,
    gh_binary: &std::path::Path,
) -> Result<String, String> {
    log::trace!("Opening pull request from {repo_path}");

    // Push current branch to remote first
    log::trace!("Pushing current branch to remote...");
    let push_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["push", "-u", "origin", "HEAD"])
        .output()
        .map_err(|e| format!("Failed to push to remote: {e}"))?;

    if !push_output.status.success() {
        let stderr = String::from_utf8_lossy(&push_output.stderr);
        // Don't fail if the branch is already up to date or already pushed
        if !stderr.contains("Everything up-to-date") && !stderr.contains("set up to track") {
            log::warn!("Push warning: {stderr}");
        }
    }
    log::trace!("Push completed");

    // Build the gh pr create command
    let mut args = vec!["pr", "create", "--fill"];

    if let Some(t) = title {
        args.push("--title");
        args.push(t);
    }

    if let Some(b) = body {
        args.push("--body");
        args.push(b);
    }

    if draft {
        args.push("--draft");
    }

    // Add --web to open in browser after creation
    args.push("--web");

    log::trace!("Running gh command with args: {:?}", args);

    let output = gh_command(gh_binary, repo_path)
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run gh pr create: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check for common errors
        if stderr.contains("already exists") {
            return Err("A pull request for this branch already exists".to_string());
        }
        if stderr.contains("no commits") || stderr.contains("nothing to compare") {
            return Err("No commits to create a pull request. Make sure you have commits that differ from the base branch.".to_string());
        }
        return Err(format!("Failed to create pull request: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    log::trace!("Successfully created pull request: {stdout}");

    Ok(stdout)
}

// =============================================================================
// PR Context Generation
// =============================================================================

/// Information needed to generate a PR prompt
#[derive(Debug, Clone, Serialize)]
pub struct PrContext {
    pub uncommitted_count: u32,
    pub current_branch: String,
    pub target_branch: String,
    pub has_upstream: bool,
    pub pr_template: Option<String>,
}

/// Get the number of uncommitted changes (staged + unstaged)
pub fn get_uncommitted_count(repo_path: &str) -> Result<u32, String> {
    let output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| format!("Failed to get git status: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout.lines().filter(|l| !l.is_empty()).count() as u32;
    Ok(count)
}

/// Check if current branch has an upstream tracking branch
pub fn has_upstream_branch(repo_path: &str) -> bool {
    wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Read PR template if it exists
pub fn get_pr_template(repo_path: &str) -> Option<String> {
    let template_path = Path::new(repo_path).join(".github/pull_request_template.md");
    std::fs::read_to_string(template_path).ok()
}

/// Generate the full PR context for the prompt
pub fn generate_pr_context(repo_path: &str, target_branch: &str) -> Result<PrContext, String> {
    Ok(PrContext {
        uncommitted_count: get_uncommitted_count(repo_path)?,
        current_branch: get_current_branch(repo_path)?,
        target_branch: target_branch.to_string(),
        has_upstream: has_upstream_branch(repo_path),
        pr_template: get_pr_template(repo_path),
    })
}

/// Read jean.json configuration from a worktree path
///
/// Returns None if the file doesn't exist or can't be parsed
pub fn read_jean_config(worktree_path: &str) -> Option<JeanConfig> {
    jean_core::read_jean_config(worktree_path)
}

/// Run a setup script in a worktree directory
///
/// Executes the script using sh -c and captures output.
/// Sets environment variables for use in the script:
/// - JEAN_WORKSPACE_PATH: Path to the newly created worktree
/// - JEAN_ROOT_PATH: Path to the repository root directory
/// - JEAN_BRANCH: Current branch name
pub fn run_setup_script(
    worktree_path: &str,
    root_path: &str,
    branch: &str,
    script: &str,
) -> Result<String, String> {
    crate::backend_runtime::script_service()
        .run_setup(worktree_path, root_path, branch, script)
        .map_err(|error| error.to_string())
}

/// Run a teardown script in a worktree directory before deletion
///
/// Executes the script using sh -c and captures output.
/// Sets environment variables for use in the script:
/// - JEAN_WORKSPACE_PATH: Path to the worktree being deleted
/// - JEAN_ROOT_PATH: Path to the repository root directory
/// - JEAN_BRANCH: Branch name of the worktree
pub fn run_teardown_script(
    worktree_path: &str,
    root_path: &str,
    branch: &str,
    script: &str,
) -> Result<String, String> {
    crate::backend_runtime::script_service()
        .run_teardown(worktree_path, root_path, branch, script)
        .map_err(|error| error.to_string())
}

/// Rebase the current branch onto a base branch from origin
///
/// This performs:
/// 1. Commits any uncommitted changes with the provided message
/// 2. Fetches from origin
/// 3. Rebases onto origin/{base_branch}
/// 4. Force pushes with lease
///
/// Returns an error message if any step fails
pub fn rebase_onto_base(
    repo_path: &str,
    base_branch: &str,
    commit_message: Option<&str>,
) -> Result<String, String> {
    log::trace!("Starting rebase onto {base_branch} in {repo_path}");

    // Step 1: Check for uncommitted changes and commit if needed
    if crate::backend_runtime::git_service()
        .has_uncommitted_changes(repo_path)
        .map_err(|error| error.to_string())?
    {
        let message = commit_message.unwrap_or("WIP: Committing changes before rebase");
        log::trace!("Committing uncommitted changes: {message}");

        // Stage all changes
        let add_output = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["add", "-A"])
            .output()
            .map_err(|e| format!("Failed to stage changes: {e}"))?;

        if !add_output.status.success() {
            let stderr = String::from_utf8_lossy(&add_output.stderr);
            return Err(format!("Failed to stage changes: {stderr}"));
        }

        // Commit
        let commit_output = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["commit", "-m", message])
            .output()
            .map_err(|e| format!("Failed to commit changes: {e}"))?;

        if !commit_output.status.success() {
            let stderr = String::from_utf8_lossy(&commit_output.stderr);
            // Not an error if nothing to commit
            if !stderr.contains("nothing to commit") {
                return Err(format!("Failed to commit changes: {stderr}"));
            }
        }
    }

    // Step 2: Fetch from origin
    log::trace!("Fetching from origin...");
    let fetch_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["fetch", "origin", base_branch])
        .output()
        .map_err(|e| format!("Failed to fetch from origin: {e}"))?;

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        return Err(format!("Failed to fetch from origin: {stderr}"));
    }

    // Step 3: Rebase onto origin/{base_branch}
    log::trace!("Rebasing onto origin/{base_branch}...");
    let rebase_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["rebase", &format!("origin/{base_branch}")])
        .output()
        .map_err(|e| format!("Failed to rebase: {e}"))?;

    if !rebase_output.status.success() {
        let stderr = String::from_utf8_lossy(&rebase_output.stderr);
        // Abort the rebase if it fails
        let _ = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["rebase", "--abort"])
            .output();
        return Err(format!(
            "Rebase failed (conflicts likely). Rebase has been aborted.\n{stderr}"
        ));
    }

    // Step 4: Force push with lease
    log::trace!("Force pushing with lease...");
    let push_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["push", "--force-with-lease"])
        .output()
        .map_err(|e| format!("Failed to push: {e}"))?;

    if !push_output.status.success() {
        let stderr = String::from_utf8_lossy(&push_output.stderr);
        // Check if branch doesn't have upstream yet, or upstream points at a
        // differently named branch (common for worktrees created from origin/main)
        if jean_core::git::push_needs_upstream_retry(&stderr) {
            // Try regular push with -u
            let push_u_output = wsl_aware_command("git", Some(Path::new(repo_path)))
                .args(["push", "-u", "origin", "HEAD"])
                .output()
                .map_err(|e| format!("Failed to push: {e}"))?;

            if !push_u_output.status.success() {
                let stderr = String::from_utf8_lossy(&push_u_output.stderr);
                return Err(format!("Failed to push: {stderr}"));
            }
        } else {
            return Err(format!("Failed to push: {stderr}"));
        }
    }

    log::trace!("Rebase completed successfully");
    Ok("Rebase completed successfully".to_string())
}

// =============================================================================
// Local Merge Operations
// =============================================================================

/// Result of a merge operation
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum MergeResult {
    /// Merge completed successfully
    Success { commit_hash: String },
    /// Merge has conflicts that need resolution
    Conflict {
        conflicting_files: Vec<String>,
        /// Diff between base and feature branch for conflicting files
        conflict_diff: String,
    },
    /// Merge failed for another reason
    Error { message: String },
}

/// Merge a feature branch into the base branch locally
///
/// This performs:
/// 1. Checks for uncommitted changes in main repo (fails if any)
/// 2. Checks out the base branch
/// 3. Pulls latest from origin (best effort)
/// 4. Merges the feature branch based on merge_type:
///    - Merge: --no-ff, creates merge commit
///    - Squash: --squash, combines all commits into one
///    - Rebase: rebases feature onto base in worktree, then fast-forward merges
///
/// On conflict, aborts the operation and returns the list of conflicting files.
///
/// # Arguments
/// * `repo_path` - Path to the main repository (NOT a worktree)
/// * `worktree_path` - Path to the worktree (for rebase - feature branch is checked out here)
/// * `feature_branch` - Name of the feature branch to merge
/// * `base_branch` - Name of the base branch to merge into
/// * `merge_type` - Type of merge operation to perform
pub fn merge_branch_to_base(
    repo_path: &str,
    worktree_path: &str,
    feature_branch: &str,
    base_branch: &str,
    merge_type: MergeType,
) -> MergeResult {
    log::trace!(
        "Merging {feature_branch} into {base_branch} in {repo_path} (type: {merge_type:?})"
    );

    // Step 1: Check for uncommitted changes in main repo - refuse to merge if any
    if crate::backend_runtime::git_service()
        .has_uncommitted_changes(repo_path)
        .unwrap_or(false)
    {
        return MergeResult::Error {
            message: "Cannot merge: there are uncommitted changes in the base branch. Please commit or stash them first.".to_string(),
        };
    }

    // Step 2: Checkout base branch
    log::trace!("Checking out {base_branch}...");
    let checkout_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["checkout", base_branch])
        .output();

    match checkout_output {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return MergeResult::Error {
                message: format!("Failed to checkout {base_branch}: {stderr}"),
            };
        }
        Err(e) => {
            return MergeResult::Error {
                message: format!("Failed to run git checkout: {e}"),
            };
        }
        _ => {}
    }

    // Step 3: Pull from origin (best effort - don't fail if no remote)
    log::trace!("Pulling latest from origin...");
    let pull_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["pull", "origin", base_branch])
        .output();

    if let Ok(output) = &pull_output {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Don't fail if no remote or already up to date
            if !stderr.contains("does not appear to be a git repository")
                && !stderr.contains("Could not read from remote")
                && !stderr.contains("couldn't find remote ref")
            {
                log::warn!("Pull warning (continuing anyway): {stderr}");
            }
        }
    }

    // Step 4: Handle based on merge type
    match merge_type {
        MergeType::Rebase => {
            // Rebase workflow: rebase in worktree, then fast-forward merge in main repo
            rebase_and_merge(repo_path, worktree_path, feature_branch, base_branch)
        }
        MergeType::Merge | MergeType::Squash => {
            // Standard merge or squash workflow
            let squash = merge_type == MergeType::Squash;
            perform_merge(repo_path, feature_branch, squash)
        }
    }
}

/// Helper function to perform a standard merge or squash merge
fn perform_merge(repo_path: &str, feature_branch: &str, squash: bool) -> MergeResult {
    log::trace!(
        "Performing {} merge...",
        if squash { "squash" } else { "standard" }
    );

    let merge_message = if squash {
        format!("Squashed commit from '{feature_branch}'")
    } else {
        format!("Merge branch '{feature_branch}'")
    };

    let merge_output = if squash {
        // --squash stages all changes but doesn't commit
        wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["merge", "--squash", feature_branch])
            .output()
    } else {
        // --no-ff creates a merge commit preserving history
        wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["merge", "--no-ff", feature_branch, "-m", &merge_message])
            .output()
    };

    match merge_output {
        Ok(output) => {
            if output.status.success() {
                // For squash merges, we need to commit the staged changes
                if squash {
                    let commit_output = wsl_aware_command("git", Some(Path::new(repo_path)))
                        .args(["commit", "-m", &merge_message])
                        .output();

                    match commit_output {
                        Ok(co) if !co.status.success() => {
                            let stderr = String::from_utf8_lossy(&co.stderr);
                            // If nothing to commit, the squash had no changes
                            if stderr.contains("nothing to commit") {
                                return MergeResult::Error {
                                    message: "No changes to merge".to_string(),
                                };
                            }
                            return MergeResult::Error {
                                message: format!("Failed to commit squashed changes: {stderr}"),
                            };
                        }
                        Err(e) => {
                            return MergeResult::Error {
                                message: format!("Failed to run git commit: {e}"),
                            };
                        }
                        _ => {}
                    }
                }

                get_head_commit_hash(repo_path)
            } else {
                handle_merge_failure(repo_path, &output.stdout, &output.stderr)
            }
        }
        Err(e) => MergeResult::Error {
            message: format!("Failed to run git merge: {e}"),
        },
    }
}

/// Helper function to perform rebase and fast-forward merge
///
/// Rebase is performed in the worktree (where feature branch is already checked out),
/// then fast-forward merge is done in the main repo.
fn rebase_and_merge(
    repo_path: &str,
    worktree_path: &str,
    feature_branch: &str,
    base_branch: &str,
) -> MergeResult {
    log::trace!("Rebasing {feature_branch} onto {base_branch} in worktree {worktree_path}...");

    // Step 1: Rebase in worktree (feature branch is already checked out there)
    let rebase_output = wsl_aware_command("git", Some(Path::new(worktree_path)))
        .args(["rebase", base_branch])
        .output();

    match rebase_output {
        Ok(output) => {
            if !output.status.success() {
                // Check for rebase conflicts
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");

                if combined.contains("CONFLICT")
                    || combined.contains("could not apply")
                    || combined.contains("fix conflicts")
                {
                    // Get list of conflicting files during rebase
                    let conflict_output = wsl_aware_command("git", Some(Path::new(worktree_path)))
                        .args(["diff", "--name-only", "--diff-filter=U"])
                        .output();

                    let conflicting_files: Vec<String> = conflict_output
                        .map(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .lines()
                                .map(|s| s.to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();

                    // Get the diff with conflict markers
                    let diff_output = wsl_aware_command("git", Some(Path::new(worktree_path)))
                        .args(["diff"])
                        .output();

                    let conflict_diff = diff_output
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_default();

                    // Abort the rebase
                    log::trace!(
                        "Rebase has conflicts in {} files, aborting...",
                        conflicting_files.len()
                    );
                    let _ = wsl_aware_command("git", Some(Path::new(worktree_path)))
                        .args(["rebase", "--abort"])
                        .output();

                    return MergeResult::Conflict {
                        conflicting_files,
                        conflict_diff,
                    };
                } else {
                    // Abort any partial rebase state
                    let _ = wsl_aware_command("git", Some(Path::new(worktree_path)))
                        .args(["rebase", "--abort"])
                        .output();

                    let error_detail = if !stderr.trim().is_empty() {
                        stderr.trim().to_string()
                    } else if !stdout.trim().is_empty() {
                        stdout.trim().to_string()
                    } else {
                        "Unknown rebase error".to_string()
                    };

                    return MergeResult::Error {
                        message: error_detail,
                    };
                }
            }
        }
        Err(e) => {
            return MergeResult::Error {
                message: format!("Failed to run git rebase: {e}"),
            };
        }
    }

    // Step 2: Fast-forward merge in main repo (base branch already checked out by caller)
    log::trace!("Rebase successful, fast-forward merging into {base_branch}...");

    let ff_merge = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["merge", "--ff-only", feature_branch])
        .output();

    match ff_merge {
        Ok(output) => {
            if output.status.success() {
                get_head_commit_hash(repo_path)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                MergeResult::Error {
                    message: format!("Fast-forward merge failed: {stderr}"),
                }
            }
        }
        Err(e) => MergeResult::Error {
            message: format!("Failed to run git merge --ff-only: {e}"),
        },
    }
}

/// Helper function to get the current HEAD commit hash
fn get_head_commit_hash(repo_path: &str) -> MergeResult {
    let hash_output = wsl_aware_command("git", Some(Path::new(repo_path)))
        .args(["rev-parse", "HEAD"])
        .output();

    let commit_hash = hash_output
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    log::trace!("Operation successful: {commit_hash}");
    MergeResult::Success { commit_hash }
}

/// Helper function to handle merge failures and extract conflict info
fn handle_merge_failure(repo_path: &str, stdout: &[u8], stderr: &[u8]) -> MergeResult {
    let stdout_str = String::from_utf8_lossy(stdout);
    let stderr_str = String::from_utf8_lossy(stderr);
    let combined = format!("{stdout_str}\n{stderr_str}");

    // Check for merge conflicts in both stdout and stderr
    if combined.contains("CONFLICT")
        || combined.contains("Automatic merge failed")
        || combined.contains("fix conflicts")
    {
        // Get list of conflicting files
        let conflict_output = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["diff", "--name-only", "--diff-filter=U"])
            .output();

        let conflicting_files: Vec<String> = conflict_output
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Get the diff with conflict markers BEFORE aborting
        let diff_output = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["diff"])
            .output();

        let conflict_diff = diff_output
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        // Abort the merge
        log::trace!(
            "Merge has conflicts in {} files, aborting...",
            conflicting_files.len()
        );
        let _ = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["merge", "--abort"])
            .output();

        MergeResult::Conflict {
            conflicting_files,
            conflict_diff,
        }
    } else {
        // Abort any partial merge state
        let _ = wsl_aware_command("git", Some(Path::new(repo_path)))
            .args(["merge", "--abort"])
            .output();

        // Create a human-friendly error message
        let error_detail = if !stderr_str.trim().is_empty() {
            stderr_str.trim().to_string()
        } else if !stdout_str.trim().is_empty() {
            stdout_str.trim().to_string()
        } else {
            "Unknown error".to_string()
        };

        MergeResult::Error {
            message: error_detail,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // get_repo_name tests
    // ========================================================================

    #[test]
    fn test_get_repo_name() {
        assert_eq!(
            get_repo_name("/Users/test/projects/my-repo").unwrap(),
            "my-repo"
        );
        assert_eq!(
            get_repo_name("/Users/test/my_project").unwrap(),
            "my_project"
        );
    }

    #[test]
    fn test_get_repo_name_with_trailing_slash() {
        // Path::file_name handles trailing slashes by ignoring them
        // "/Users/test/my-repo/" -> file_name is None for trailing slash
        let result = get_repo_name("/");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_repo_name_single_component() {
        assert_eq!(get_repo_name("my-repo").unwrap(), "my-repo");
    }

    // ========================================================================
    // get_user_shell tests (testing the pure logic)
    // ========================================================================

    #[test]
    fn test_shell_login_support_detection() {
        // Test the login support detection logic
        let supports_login = |shell: &str| -> bool {
            shell.ends_with("bash")
                || shell.ends_with("zsh")
                || shell.ends_with("fish")
                || shell.ends_with("ksh")
                || shell.ends_with("tcsh")
        };

        assert!(supports_login("/bin/bash"));
        assert!(supports_login("/bin/zsh"));
        assert!(supports_login("/usr/local/bin/fish"));
        assert!(supports_login("/bin/ksh"));
        assert!(supports_login("/bin/tcsh"));
        assert!(!supports_login("/bin/sh"));
        assert!(!supports_login("/bin/dash"));
    }

    // ========================================================================
    // git push upstream retry tests
    // ========================================================================

    #[test]
    fn test_git_push_needs_upstream_retry_for_no_upstream() {
        let stderr = "fatal: The current branch feature has no upstream branch.";
        assert!(jean_core::git::push_needs_upstream_retry(stderr));
    }

    #[test]
    fn test_git_push_needs_upstream_retry_for_simple_mismatch() {
        let stderr = "fatal: The upstream branch of your current branch does not match\n\
                      the name of your current branch.  To push to the upstream branch\n\
                      on the remote, use\n\n\
                      git push origin HEAD:main\n\n\
                      To push to the branch of the same name on the remote, use\n\n\
                      git push origin HEAD\n\n\
                      To choose either option permanently, see push.default in git-config.\n\
                      fatal: push.default is set to simple";
        assert!(jean_core::git::push_needs_upstream_retry(stderr));
    }

    #[test]
    fn retryable_worktree_create_error_detects_git_config_lock() {
        let stderr = "Preparing worktree (new branch 'issue-10593')\n\
                      error: could not lock config file .git/config: File exists\n\
                      error: unable to write upstream branch configuration";

        assert!(is_retryable_worktree_create_error(stderr));
    }

    #[test]
    fn retryable_worktree_create_error_rejects_real_conflicts() {
        assert!(!is_retryable_worktree_create_error(
            "fatal: a branch named 'issue-10593' already exists"
        ));
        assert!(!is_retryable_worktree_create_error(
            "fatal: '/tmp/project/issue-10593' already exists"
        ));
    }

    #[test]
    fn test_git_push_needs_upstream_retry_ignores_permission_errors() {
        let stderr = "remote: Permission to owner/repo.git denied to user.\n\
                      fatal: unable to access 'https://github.com/owner/repo.git/': \
                      The requested URL returned error: 403";
        assert!(!jean_core::git::push_needs_upstream_retry(stderr));
    }

    // ========================================================================
    // RepoIdentifier tests
    // ========================================================================

    #[test]
    fn test_repo_identifier_to_key() {
        let id = RepoIdentifier {
            owner: "heyandras".to_string(),
            repo: "jean".to_string(),
        };
        assert_eq!(id.to_key(), "heyandras-jean");
    }

    #[test]
    fn test_repo_identifier_to_key_with_hyphen_in_name() {
        let id = RepoIdentifier {
            owner: "my-org".to_string(),
            repo: "my-project".to_string(),
        };
        assert_eq!(id.to_key(), "my-org-my-project");
    }
}
