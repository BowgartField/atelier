use crate::{BackendError, BackendErrorCode};
use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Component, Path};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const EMPTY_TREE_HASH: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const COMMIT_MARKER: &str = "COMMIT\0";

#[derive(Debug, Clone, Serialize)]
pub struct GitRemote {
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorktreeFile {
    pub relative_path: String,
    pub extension: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub line_type: String,
    pub content: String,
    pub old_line_number: Option<u32>,
    pub new_line_number: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffHunk {
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub is_binary: bool,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitDiff {
    pub diff_type: String,
    pub base_ref: String,
    pub target_ref: String,
    pub total_additions: u32,
    pub total_deletions: u32,
    pub files: Vec<DiffFile>,
    pub raw_patch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub message: String,
    pub author_name: String,
    pub author_date: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitHistoryResult {
    pub commits: Vec<CommitInfo>,
    pub total_count: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPushResponse {
    pub output: String,
    pub fell_back: bool,
    pub permission_denied: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevertCommitResponse {
    pub commit_hash: String,
    pub commit_message: String,
}

pub type GitRunner = fn(&Path, &[&str]) -> Result<Output, BackendError>;

#[derive(Clone, Copy)]
pub struct GitService {
    runner: GitRunner,
}

impl Default for GitService {
    fn default() -> Self {
        Self { runner: native_git }
    }
}

impl GitService {
    pub fn new(runner: GitRunner) -> Self {
        Self { runner }
    }

    pub(crate) fn run(self, path: &Path, args: &[&str]) -> Result<Output, BackendError> {
        (self.runner)(path, args)
    }

    fn ok(self, path: &Path, args: &[&str], operation: &str) -> Result<(), BackendError> {
        let output = self.run(path, args)?;
        if output.status.success() {
            Ok(())
        } else {
            Err(git_failure(output, operation))
        }
    }

    pub(crate) fn text(self, path: &Path, args: &[&str]) -> Result<String, BackendError> {
        Ok(self.text_preserve(path, args)?.trim().to_owned())
    }

    fn text_preserve(self, path: &Path, args: &[&str]) -> Result<String, BackendError> {
        let output = self.run(path, args)?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(git_failure(output, "git"))
        }
    }

    pub fn is_repository(self, path: &str) -> Result<bool, BackendError> {
        let root = Path::new(path);
        if !root.is_dir() {
            return Ok(false);
        }
        Ok(self
            .run(root, &["rev-parse", "--git-dir"])?
            .status
            .success())
    }

    pub fn current_branch(self, path: &str) -> Result<String, BackendError> {
        self.text(Path::new(path), &["symbolic-ref", "--short", "HEAD"])
    }

    pub fn repository_key(self, path: &str) -> Result<String, BackendError> {
        let remote = self.text(Path::new(path), &["remote", "get-url", "origin"])?;
        parse_github_repository_key(&remote).ok_or_else(|| {
            invalid(format!(
                "Origin is not a supported GitHub repository URL: {remote}"
            ))
        })
    }

    pub fn init_repository(self, path: &str) -> Result<(), BackendError> {
        let root = Path::new(path);
        if root.exists() && !root.is_dir() {
            return Err(invalid(format!("Path is not a directory: {path}")));
        }
        std::fs::create_dir_all(root)?;

        if root.join(".git").exists() {
            if self
                .run(root, &["rev-parse", "HEAD"])
                .is_ok_and(|output| output.status.success())
            {
                return Err(invalid("Directory is already a git repository"));
            }
        } else {
            self.ok(root, &["init"], "git init")?;
        }

        std::fs::write(root.join(".gitkeep"), b"")?;
        self.ok(root, &["add", ".gitkeep"], "git add")?;
        self.ok(
            root,
            &["commit", "-m", "jean's init vibe commit"],
            "git commit",
        )
    }

    pub fn clone_repository(self, url: &str, destination: &str) -> Result<(), BackendError> {
        let url = url.trim();
        if !(url.starts_with("https://")
            || url.starts_with("http://")
            || url.starts_with("git@")
            || url.starts_with("ssh://"))
        {
            return Err(invalid(
                "Invalid git URL. Use HTTPS (https://...) or SSH (git@...) format.",
            ));
        }
        let destination_path = Path::new(destination);
        if destination_path.exists() {
            return Err(invalid(format!(
                "Destination already exists: {destination}"
            )));
        }
        let parent = destination_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        self.ok(parent, &["clone", url, destination], "git clone")
    }

    pub fn project_branches(self, path: &str) -> Result<Vec<String>, BackendError> {
        let root = Path::new(path);
        let _ = self.run(root, &["fetch", "origin"]);
        let mut branches = self
            .text(root, &["branch", "-r", "--format=%(refname:short)"])?
            .lines()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .filter_map(|branch| {
                if branch.contains("HEAD") || branch == "origin" {
                    None
                } else {
                    Some(branch.strip_prefix("origin/").unwrap_or(branch).to_owned())
                }
            })
            .collect::<Vec<_>>();
        if branches.is_empty() {
            branches = self.branches(path)?;
        }
        branches.sort();
        branches.dedup();
        Ok(branches)
    }

    pub fn branch_exists(self, path: &str, branch: &str) -> bool {
        self.run(
            Path::new(path),
            &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
        )
        .is_ok_and(|output| output.status.success())
    }

    pub fn remote_branch_exists(self, path: &str, branch: &str) -> bool {
        self.run(
            Path::new(path),
            &[
                "rev-parse",
                "--verify",
                &format!("refs/remotes/origin/{branch}"),
            ],
        )
        .is_ok_and(|output| output.status.success())
    }

    pub fn has_commits(self, path: &str) -> bool {
        self.run(Path::new(path), &["rev-parse", "HEAD"])
            .is_ok_and(|output| output.status.success())
    }

    pub fn valid_base_branch(
        self,
        path: &str,
        preferred_branch: &str,
    ) -> Result<String, BackendError> {
        if !self.has_commits(path) {
            return Err(invalid(
                "Cannot create worktree: repository has no commits yet. Please make an initial commit first.",
            ));
        }
        if self.branch_exists(path, preferred_branch)
            || self.remote_branch_exists(path, preferred_branch)
        {
            return Ok(preferred_branch.to_string());
        }
        for fallback in ["main", "master"] {
            if self.branch_exists(path, fallback) {
                return Ok(fallback.to_string());
            }
        }
        self.current_branch_with_fallback(path)
    }

    pub fn create_worktree(
        self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
        base: &str,
    ) -> Result<(), BackendError> {
        self.create_worktree_with_args(
            repo_path,
            worktree_path,
            &["worktree", "add", "-b", branch, worktree_path, base],
        )
    }

    pub fn create_worktree_from_existing_branch(
        self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
    ) -> Result<(), BackendError> {
        self.create_worktree_with_args(
            repo_path,
            worktree_path,
            &["worktree", "add", worktree_path, branch],
        )
    }

    fn create_worktree_with_args(
        self,
        repo_path: &str,
        worktree_path: &str,
        args: &[&str],
    ) -> Result<(), BackendError> {
        if let Some(parent) = Path::new(worktree_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = Path::new(repo_path);
        for attempt in 1..=4 {
            let _ = self.run(root, &["worktree", "prune"]);
            let output = self.run(root, args)?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if attempt < 4 && retryable_worktree_create_error(&stderr) {
                std::thread::sleep(Duration::from_millis(150 * attempt));
                continue;
            }
            return Err(BackendError::new(
                BackendErrorCode::Io,
                format!("Failed to create worktree: {}", stderr.trim()),
            ));
        }
        Err(BackendError::new(
            BackendErrorCode::Io,
            "Failed to create worktree: retry attempts exhausted",
        ))
    }

    pub fn remove_worktree(self, repo_path: &str, worktree_path: &str) -> Result<(), BackendError> {
        let root = Path::new(repo_path);
        let target = Path::new(worktree_path);
        if same_path(root, target) || self.is_main_worktree(repo_path, worktree_path) {
            return Err(invalid(format!(
                "Refusing to remove main working tree at {worktree_path}"
            )));
        }
        let _ = self.run(root, &["worktree", "prune"]);
        let output = self.run(root, &["worktree", "remove", worktree_path, "--force"])?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stale = stderr.contains("is not a working tree")
                || stderr.contains("does not exist")
                || stderr.contains("No such file or directory");
            if !stale && !target.exists() {
                return Err(BackendError::new(
                    BackendErrorCode::Io,
                    format!("Failed to remove worktree: {}", stderr.trim()),
                ));
            }
        }
        if target.exists() {
            remove_dir_all_with_retry(target)?;
        }
        let _ = self.run(root, &["worktree", "prune"]);
        Ok(())
    }

    pub fn delete_branch(self, repo_path: &str, branch: &str) -> Result<(), BackendError> {
        let output = self.run(Path::new(repo_path), &["branch", "-D", branch])?;
        if output.status.success() || String::from_utf8_lossy(&output.stderr).contains("not found")
        {
            Ok(())
        } else {
            Err(git_failure(output, "git branch -D"))
        }
    }

    fn current_branch_with_fallback(self, path: &str) -> Result<String, BackendError> {
        self.current_branch(path)
            .or_else(|_| self.text(Path::new(path), &["rev-parse", "--abbrev-ref", "HEAD"]))
    }

    fn is_main_worktree(self, repo_path: &str, worktree_path: &str) -> bool {
        let Ok(output) = self.text(Path::new(repo_path), &["worktree", "list", "--porcelain"])
        else {
            return false;
        };
        output
            .lines()
            .find_map(|line| line.strip_prefix("worktree "))
            .is_some_and(|main| same_path(Path::new(main), Path::new(worktree_path)))
    }

    pub fn has_uncommitted_changes(self, path: &str) -> Result<bool, BackendError> {
        Ok(!self
            .text(Path::new(path), &["status", "--porcelain"])?
            .is_empty())
    }

    pub fn branches(self, path: &str) -> Result<Vec<String>, BackendError> {
        Ok(self
            .text(Path::new(path), &["branch", "--format=%(refname:short)"])?
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect())
    }

    pub fn remotes(self, path: &str) -> Result<Vec<GitRemote>, BackendError> {
        Ok(self
            .text(Path::new(path), &["remote"])?
            .lines()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|name| GitRemote {
                name: name.to_owned(),
            })
            .collect())
    }

    pub fn remove_remote(self, path: &str, remote: &str) -> Result<(), BackendError> {
        self.ok(
            Path::new(path),
            &["remote", "remove", remote],
            "remove remote",
        )
    }

    pub fn pull(
        self,
        path: &str,
        branch: &str,
        remote: Option<&str>,
    ) -> Result<String, BackendError> {
        let remote = remote.unwrap_or("origin");
        self.ok(Path::new(path), &["fetch", remote, branch], "git fetch")?;
        let root = Path::new(path);
        let merge = self.run(root, &["merge", &format!("{remote}/{branch}")])?;
        if merge.status.success() {
            return Ok(String::from_utf8_lossy(&merge.stdout).into_owned());
        }
        let stdout = String::from_utf8_lossy(&merge.stdout);
        let stderr = String::from_utf8_lossy(&merge.stderr);
        if stdout.contains("CONFLICT") || stdout.contains("Automatic merge failed") {
            let conflicts = self
                .text(root, &["diff", "--name-only", "--diff-filter=U"])
                .unwrap_or_default();
            return Err(BackendError::new(
                BackendErrorCode::Io,
                format!(
                    "Merge conflicts in: {conflicts}. Resolve manually or run 'git merge --abort'"
                ),
            ));
        }
        Err(BackendError::new(
            BackendErrorCode::Io,
            if stderr.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                stderr.trim().to_string()
            },
        ))
    }

    pub fn stash(self, path: &str) -> Result<String, BackendError> {
        self.text(Path::new(path), &["stash", "--include-untracked"])
    }

    pub fn stash_pop(self, path: &str) -> Result<String, BackendError> {
        self.text(Path::new(path), &["stash", "pop"])
    }

    pub fn push(self, path: &str, remote: Option<&str>) -> Result<GitPushResponse, BackendError> {
        let remote = remote.unwrap_or("origin");
        let root = Path::new(path);
        let branch = self
            .text(root, &["symbolic-ref", "--short", "HEAD"])
            .or_else(|_| self.text(root, &["rev-parse", "--abbrev-ref", "HEAD"]))?;
        let expected = format!("{remote}/{branch}");
        let upstream = self
            .text(root, &["rev-parse", "--abbrev-ref", "@{upstream}"])
            .ok();
        let args = if upstream.as_deref() == Some(expected.as_str()) {
            vec!["push", remote]
        } else {
            vec!["push", "-u", remote, "HEAD"]
        };
        let output = self.run(root, &args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if args.len() == 2 && push_needs_upstream_retry(&stderr) {
                let retry = self.run(root, &["push", "-u", remote, "HEAD"])?;
                if retry.status.success() {
                    return Ok(push_response(retry));
                }
                return Err(git_failure(retry, "git push -u"));
            }
            return Err(git_failure(output, "git push"));
        }
        Ok(push_response(output))
    }

    pub fn commit(
        self,
        path: &str,
        message: &str,
        stage_all: bool,
    ) -> Result<String, BackendError> {
        let root = Path::new(path);
        if message.trim().is_empty() {
            return Err(invalid("Commit message cannot be empty"));
        }
        if stage_all {
            self.ok(root, &["add", "-A"], "git add")?;
        }
        if self.text(root, &["status", "--porcelain"])?.is_empty() {
            return Err(invalid("Nothing to commit, working tree clean"));
        }
        if self
            .run(root, &["diff", "--cached", "--quiet"])?
            .status
            .success()
        {
            return Err(invalid(
                "No staged changes to commit. Stage changes first or enable stage all.",
            ));
        }
        self.ok(root, &["commit", "-m", message], "git commit")?;
        self.text(root, &["rev-parse", "HEAD"])
    }

    pub fn revert_last_commit(self, path: &str) -> Result<RevertCommitResponse, BackendError> {
        let root = Path::new(path);
        let metadata = self.text(root, &["log", "-1", "--format=%H%n%s"])?;
        let mut lines = metadata.lines();
        let commit_hash = lines.next().unwrap_or_default().to_owned();
        let commit_message = lines.next().unwrap_or_default().to_owned();
        if commit_hash.is_empty() {
            return Err(invalid("No commits to revert"));
        }
        self.ok(root, &["reset", "--soft", "HEAD~1"], "git reset")?;
        Ok(RevertCommitResponse {
            commit_hash,
            commit_message,
        })
    }

    pub fn revert_file(
        self,
        path: &str,
        file_path: &str,
        status: &str,
    ) -> Result<(), BackendError> {
        let relative = safe_relative_path(file_path)?;
        let root = Path::new(path);
        match status {
            "modified" | "deleted" => {
                self.ok(root, &["checkout", "HEAD", "--", file_path], "revert file")
            }
            "added" => {
                let _ = self.run(root, &["reset", "HEAD", "--", file_path]);
                remove_file(root.join(relative), "remove added file")
            }
            "renamed" => {
                if self
                    .ok(
                        root,
                        &["checkout", "HEAD", "--", file_path],
                        "revert rename",
                    )
                    .is_err()
                {
                    let _ = self.run(root, &["reset", "HEAD", "--", file_path]);
                    remove_file(root.join(relative), "remove renamed file")?;
                }
                Ok(())
            }
            _ => Err(invalid(format!("Unknown file status: {status}"))),
        }
    }

    pub fn list_files(self, path: &str, max: usize) -> Result<Vec<WorktreeFile>, BackendError> {
        let root = Path::new(path);
        if !root.is_dir() {
            return Err(invalid("Worktree path is not a directory"));
        }
        let mut files = Vec::new();
        for entry in WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .build()
        {
            if files.len() >= max {
                break;
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path == root || path.components().any(|part| part.as_os_str() == ".git") {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let is_dir = path.is_dir();
            files.push(WorktreeFile {
                relative_path: relative.to_string_lossy().into_owned(),
                extension: if is_dir {
                    String::new()
                } else {
                    path.extension()
                        .and_then(|v| v.to_str())
                        .unwrap_or_default()
                        .to_owned()
                },
                is_dir,
            });
        }
        files.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        Ok(files)
    }

    pub fn identity(self) -> GitIdentity {
        let root = std::env::temp_dir();
        GitIdentity {
            name: self
                .text(&root, &["config", "--global", "user.name"])
                .ok()
                .filter(|v| !v.is_empty()),
            email: self
                .text(&root, &["config", "--global", "user.email"])
                .ok()
                .filter(|v| !v.is_empty()),
        }
    }

    pub fn set_identity(self, name: &str, email: &str) -> Result<(), BackendError> {
        if name.trim().is_empty() || email.trim().is_empty() {
            return Err(invalid("Git name and email are required"));
        }
        let root = std::env::temp_dir();
        self.ok(
            &root,
            &["config", "--global", "user.name", name],
            "set git user.name",
        )?;
        self.ok(
            &root,
            &["config", "--global", "user.email", email],
            "set git user.email",
        )
    }

    pub fn diff(
        self,
        path: &str,
        diff_type: &str,
        base: Option<&str>,
    ) -> Result<GitDiff, BackendError> {
        let root = Path::new(path);
        let has_head = self
            .run(root, &["rev-parse", "--verify", "HEAD"])
            .is_ok_and(|o| o.status.success());
        let base = base.unwrap_or("main");
        let (base_ref, target_ref, args) = match diff_type {
            "uncommitted" => (
                if has_head { "HEAD" } else { "empty tree" }.to_owned(),
                "working directory".to_owned(),
                vec![
                    "diff".to_owned(),
                    if has_head { "HEAD" } else { EMPTY_TREE_HASH }.to_owned(),
                    "--unified=3".to_owned(),
                ],
            ),
            "branch" => (
                format!("origin/{base}"),
                "HEAD".to_owned(),
                vec![
                    "diff".to_owned(),
                    "--unified=3".to_owned(),
                    format!("origin/{base}...HEAD"),
                ],
            ),
            _ => return Err(invalid(format!("Invalid diff_type: {diff_type}"))),
        };
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let mut raw_patch = self.text_preserve(root, &refs)?;
        let untracked = if diff_type == "uncommitted" {
            let (patch, paths) = self.untracked_patch(root);
            raw_patch.push_str(&patch);
            paths
        } else {
            BTreeSet::new()
        };
        let mut diff = build_diff(diff_type, base_ref, target_ref, raw_patch);
        for file in &mut diff.files {
            if untracked.contains(&file.path) {
                file.status = "untracked".to_string();
            }
        }
        for path in untracked {
            if !diff.files.iter().any(|file| file.path == path) {
                diff.files.push(DiffFile {
                    path,
                    old_path: None,
                    status: "untracked".to_string(),
                    additions: 0,
                    deletions: 0,
                    is_binary: true,
                    hunks: Vec::new(),
                });
            }
        }
        Ok(diff)
    }

    pub fn commit_diff(self, path: &str, sha: &str) -> Result<GitDiff, BackendError> {
        let root = Path::new(path);
        let parent = format!("{sha}^");
        let range = format!("{parent}..{sha}");
        let raw = self
            .text_preserve(root, &["diff", "--unified=3", &range])
            .or_else(|_| {
                self.text_preserve(root, &["diff-tree", "-p", "--unified=3", "--root", sha])
            })?;
        Ok(build_diff("commit", parent, sha.to_owned(), raw))
    }

    pub fn commit_history(
        self,
        path: &str,
        branch: Option<&str>,
        limit: u32,
        skip: u32,
    ) -> Result<CommitHistoryResult, BackendError> {
        let root = Path::new(path);
        let branch = branch.unwrap_or("HEAD");
        let total_count = self
            .text(root, &["rev-list", "--count", branch, "--"])
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let output = self.text_preserve(
            root,
            &[
                "log",
                &format!("-{}", limit.saturating_add(1)),
                &format!("--skip={skip}"),
                "--format=COMMIT%x00%H%x00%h%x00%s%x00%an%x00%aI",
                "--numstat",
                "-m",
                "--first-parent",
                branch,
                "--",
            ],
        )?;
        let mut commits = Vec::new();
        let mut current: Option<CommitInfo> = None;
        for line in output.lines() {
            if let Some(value) = line.strip_prefix(COMMIT_MARKER) {
                if let Some(commit) = current.take() {
                    commits.push(commit);
                }
                let fields = value.split('\0').collect::<Vec<_>>();
                if fields.len() >= 5 {
                    current = Some(CommitInfo {
                        sha: fields[0].to_owned(),
                        short_sha: fields[1].to_owned(),
                        message: fields[2].to_owned(),
                        author_name: fields[3].to_owned(),
                        author_date: fields[4].to_owned(),
                        additions: 0,
                        deletions: 0,
                    });
                }
            } else if let Some(commit) = current.as_mut() {
                let fields = line.split('\t').collect::<Vec<_>>();
                if fields.len() >= 3 && fields[0] != "-" {
                    commit.additions += fields[0].parse::<u32>().unwrap_or(0);
                    commit.deletions += fields[1].parse::<u32>().unwrap_or(0);
                }
            }
        }
        if let Some(commit) = current {
            commits.push(commit);
        }
        let has_more = commits.len() > limit as usize;
        commits.truncate(limit as usize);
        Ok(CommitHistoryResult {
            commits,
            total_count,
            has_more,
        })
    }

    fn untracked_patch(self, root: &Path) -> (String, BTreeSet<String>) {
        let Ok(paths) = self.text(root, &["ls-files", "--others", "--exclude-standard"]) else {
            return (String::new(), BTreeSet::new());
        };
        let mut patch = String::new();
        let mut included = BTreeSet::new();
        for relative in paths.lines().filter(|value| !value.is_empty()) {
            included.insert(relative.to_string());
            let Ok(content) = std::fs::read_to_string(root.join(relative)) else {
                continue;
            };
            let lines = content.lines().collect::<Vec<_>>();
            patch.push_str(&format!(
                "diff --git a/{relative} b/{relative}\nnew file mode 100644\n--- /dev/null\n+++ b/{relative}\n@@ -0,0 +1,{} @@\n",
                lines.len()
            ));
            for line in lines {
                patch.push('+');
                patch.push_str(line);
                patch.push('\n');
            }
        }
        (patch, included)
    }
}

fn parse_github_repository_key(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = trimmed.strip_prefix("ssh://git@github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("https://github.com/") {
        path
    } else {
        trimmed.strip_prefix("http://github.com/")?
    };
    let (owner, repository) = path.split_once('/')?;
    if owner.is_empty() || repository.is_empty() || repository.contains('/') {
        return None;
    }
    Some(format!("{owner}-{repository}"))
}

fn build_diff(kind: &str, base_ref: String, target_ref: String, raw_patch: String) -> GitDiff {
    let files = parse_unified_diff(&raw_patch);
    GitDiff {
        diff_type: kind.to_owned(),
        base_ref,
        target_ref,
        total_additions: files.iter().map(|f| f.additions).sum(),
        total_deletions: files.iter().map(|f| f.deletions).sum(),
        files,
        raw_patch,
    }
}

fn parse_unified_diff(raw: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut file: Option<DiffFile> = None;
    let mut hunk: Option<DiffHunk> = None;
    let mut old_line = 0;
    let mut new_line = 0;
    for line in raw.lines() {
        if let Some(paths) = line.strip_prefix("diff --git a/") {
            flush_hunk(&mut file, &mut hunk);
            if let Some(previous) = file.take() {
                files.push(previous);
            }
            let path = paths
                .split_once(" b/")
                .map(|(_, path)| path)
                .unwrap_or(paths);
            file = Some(DiffFile {
                path: path.to_owned(),
                old_path: None,
                status: "modified".to_owned(),
                additions: 0,
                deletions: 0,
                is_binary: false,
                hunks: Vec::new(),
            });
        } else if line.starts_with("new file mode") {
            if let Some(f) = file.as_mut() {
                f.status = "added".to_owned();
            }
        } else if line.starts_with("deleted file mode") {
            if let Some(f) = file.as_mut() {
                f.status = "deleted".to_owned();
            }
        } else if let Some(path) = line.strip_prefix("rename from ") {
            if let Some(f) = file.as_mut() {
                f.status = "renamed".to_owned();
                f.old_path = Some(path.to_owned());
            }
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            if let Some(f) = file.as_mut() {
                f.is_binary = true;
            }
        } else if line.starts_with("@@ ") {
            flush_hunk(&mut file, &mut hunk);
            if let Some((os, oc, ns, nc)) = parse_hunk_header(line) {
                old_line = os;
                new_line = ns;
                hunk = Some(DiffHunk {
                    header: line.to_owned(),
                    old_start: os,
                    old_lines: oc,
                    new_start: ns,
                    new_lines: nc,
                    lines: Vec::new(),
                });
            }
        } else if let (Some(f), Some(h)) = (file.as_mut(), hunk.as_mut()) {
            if let Some(content) = line.strip_prefix('+') {
                f.additions += 1;
                h.lines.push(DiffLine {
                    line_type: "addition".to_owned(),
                    content: content.to_owned(),
                    old_line_number: None,
                    new_line_number: Some(new_line),
                });
                new_line += 1;
            } else if let Some(content) = line.strip_prefix('-') {
                f.deletions += 1;
                h.lines.push(DiffLine {
                    line_type: "deletion".to_owned(),
                    content: content.to_owned(),
                    old_line_number: Some(old_line),
                    new_line_number: None,
                });
                old_line += 1;
            } else if let Some(content) = line.strip_prefix(' ') {
                h.lines.push(DiffLine {
                    line_type: "context".to_owned(),
                    content: content.to_owned(),
                    old_line_number: Some(old_line),
                    new_line_number: Some(new_line),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }
    flush_hunk(&mut file, &mut hunk);
    if let Some(file) = file {
        files.push(file);
    }
    files
}

fn flush_hunk(file: &mut Option<DiffFile>, hunk: &mut Option<DiffHunk>) {
    if let (Some(file), Some(hunk)) = (file.as_mut(), hunk.take()) {
        file.hunks.push(hunk);
    }
}
fn parse_hunk_header(value: &str) -> Option<(u32, u32, u32, u32)> {
    let mut p = value.split_whitespace();
    p.next()?;
    let old = parse_range(p.next()?.strip_prefix('-')?)?;
    let new = parse_range(p.next()?.strip_prefix('+')?)?;
    Some((old.0, old.1, new.0, new.1))
}
fn parse_range(value: &str) -> Option<(u32, u32)> {
    let mut p = value.split(',');
    Some((
        p.next()?.parse().ok()?,
        p.next().map(str::parse).transpose().ok()?.unwrap_or(1),
    ))
}

fn safe_relative_path(value: &str) -> Result<&Path, BackendError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|p| {
            matches!(
                p,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        Err(invalid("File path must stay inside the worktree"))
    } else {
        Ok(path)
    }
}
fn remove_file(path: impl AsRef<Path>, operation: &str) -> Result<(), BackendError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(BackendError::new(
            BackendErrorCode::Io,
            format!("{operation} failed: {e}"),
        )),
    }
}
fn native_git(path: &Path, args: &[&str]) -> Result<Output, BackendError> {
    silent_command("git")
        .current_dir(path)
        .args(args)
        .output()
        .map_err(|e| BackendError::new(BackendErrorCode::Io, format!("Failed to run git: {e}")))
}
#[cfg(test)]
fn git_ok(path: &Path, args: &[&str], operation: &str) -> Result<(), BackendError> {
    let output = native_git(path, args)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(output, operation))
    }
}
fn git_failure(output: Output, operation: &str) -> BackendError {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let details = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    BackendError::new(
        BackendErrorCode::Io,
        format!("{operation} failed: {details}"),
    )
}

fn push_response(output: Output) -> GitPushResponse {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    GitPushResponse {
        output: if stdout.is_empty() { stderr } else { stdout }.into_owned(),
        fell_back: false,
        permission_denied: false,
    }
}

pub fn push_needs_upstream_retry(stderr: &str) -> bool {
    stderr.contains("has no upstream branch")
        || stderr.contains("upstream branch of your current branch does not match")
        || stderr.contains("push.default is set to simple")
}

pub fn retryable_worktree_create_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("could not lock config file")
        || lower.contains("config.lock")
        || (lower.contains(".git/config") && lower.contains("file exists"))
        || lower.contains("unable to write upstream branch configuration")
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn remove_dir_all_with_retry(path: &Path) -> Result<(), BackendError> {
    let mut last_error = None;
    for attempt in 1..=5 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 5 {
                    std::thread::sleep(Duration::from_millis(200 * attempt));
                }
            }
        }
    }
    Err(BackendError::new(
        BackendErrorCode::Io,
        format!(
            "Failed to remove worktree directory {}: {}",
            path.display(),
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        ),
    ))
}
fn invalid(message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorCode::InvalidArgument, message)
}

#[cfg(windows)]
fn silent_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command.creation_flags(0x08000000);
    command
}
#[cfg(not(windows))]
fn silent_command(program: &str) -> Command {
    Command::new(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RUNNER_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_runner(path: &Path, args: &[&str]) -> Result<Output, BackendError> {
        RUNNER_CALLS.fetch_add(1, Ordering::SeqCst);
        native_git(path, args)
    }
    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init"], "init").unwrap();
        git_ok(dir.path(), &["config", "user.name", "Jean Test"], "name").unwrap();
        git_ok(
            dir.path(),
            &["config", "user.email", "test@jean.local"],
            "email",
        )
        .unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();
        git_ok(dir.path(), &["add", "."], "add").unwrap();
        git_ok(dir.path(), &["commit", "-m", "initial"], "commit").unwrap();
        dir
    }
    #[test]
    fn history_diff_commit_and_revert_are_compatible() {
        let repo = repository();
        let path = repo.path().to_str().unwrap();
        let service = GitService::default();
        std::fs::write(repo.path().join("hello.txt"), "hello\nworld\n").unwrap();
        assert!(service.has_uncommitted_changes(path).unwrap());
        assert_eq!(
            service
                .diff(path, "uncommitted", None)
                .unwrap()
                .total_additions,
            1
        );
        let sha = service.commit(path, "second", true).unwrap();
        let history = service.commit_history(path, None, 1, 0).unwrap();
        assert_eq!(history.total_count, 2);
        assert!(history.has_more);
        assert_eq!(service.commit_diff(path, &sha).unwrap().total_additions, 1);
        assert_eq!(service.revert_last_commit(path).unwrap().commit_hash, sha);
    }
    #[test]
    fn untracked_files_are_included_and_path_escape_is_rejected() {
        let repo = repository();
        let path = repo.path().to_str().unwrap();
        std::fs::write(repo.path().join("new.txt"), "new\n").unwrap();
        let service = GitService::default();
        assert_eq!(
            service.diff(path, "uncommitted", None).unwrap().files[0].status,
            "untracked"
        );
        assert!(service
            .revert_file(path, "../outside.txt", "added")
            .is_err());
    }

    #[test]
    fn injected_runner_is_used_by_the_shared_business_logic() {
        let repo = repository();
        RUNNER_CALLS.store(0, Ordering::SeqCst);
        GitService::new(counting_runner)
            .branches(repo.path().to_str().unwrap())
            .unwrap();
        assert_eq!(RUNNER_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn repository_lifecycle_and_project_branches_use_the_shared_service() {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"], "init").unwrap();
        git_ok(dir.path(), &["config", "user.name", "Jean Test"], "name").unwrap();
        git_ok(
            dir.path(),
            &["config", "user.email", "test@jean.local"],
            "email",
        )
        .unwrap();

        let path = dir.path().to_str().unwrap();
        let service = GitService::default();
        service.init_repository(path).unwrap();

        assert!(service.is_repository(path).unwrap());
        assert_eq!(service.current_branch(path).unwrap(), "main");
        assert_eq!(service.project_branches(path).unwrap(), vec!["main"]);
        assert!(dir.path().join(".gitkeep").is_file());
        assert_eq!(
            service
                .text(dir.path(), &["log", "-1", "--format=%s"])
                .unwrap(),
            "jean's init vibe commit"
        );
        assert!(service.init_repository(path).is_err());
    }

    #[test]
    fn clone_repository_validates_the_desktop_url_contract() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("clone");
        let error = GitService::default()
            .clone_repository("/tmp/local-repository", destination.to_str().unwrap())
            .unwrap_err();
        assert_eq!(error.code, BackendErrorCode::InvalidArgument);
        assert!(!destination.exists());
    }

    #[test]
    fn worktree_lifecycle_is_shared_and_protects_the_main_checkout() {
        let repo = repository();
        let worktree = repo
            .path()
            .parent()
            .unwrap()
            .join(format!("jean-core-worktree-{}", uuid::Uuid::new_v4()));
        let service = GitService::default();
        let repo_path = repo.path().to_str().unwrap();
        let worktree_path = worktree.to_str().unwrap();
        let base = service.current_branch(repo_path).unwrap();

        service
            .create_worktree(repo_path, worktree_path, "feature/shared", &base)
            .unwrap();
        assert!(worktree.is_dir());
        assert!(service.branch_exists(repo_path, "feature/shared"));
        assert!(service.remove_worktree(repo_path, repo_path).is_err());
        service.remove_worktree(repo_path, worktree_path).unwrap();
        service.delete_branch(repo_path, "feature/shared").unwrap();
        assert!(!worktree.exists());
        assert!(!service.branch_exists(repo_path, "feature/shared"));
    }

    #[test]
    fn upstream_retry_only_matches_configuration_failures() {
        assert!(push_needs_upstream_retry(
            "fatal: The current branch feature has no upstream branch."
        ));
        assert!(push_needs_upstream_retry(
            "the upstream branch of your current branch does not match"
        ));
        assert!(!push_needs_upstream_retry("Permission denied (publickey)"));
    }
}
