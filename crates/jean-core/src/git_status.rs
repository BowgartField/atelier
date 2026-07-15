use crate::{BackendError, GitService};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveWorktreeInfo {
    pub worktree_id: String,
    pub worktree_path: String,
    pub base_branch: String,
    pub pr_number: Option<u32>,
    pub pr_url: Option<String>,
    pub pr_push_remote: Option<String>,
    pub pr_push_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitBranchStatus {
    pub worktree_id: String,
    pub current_branch: String,
    pub base_branch: String,
    pub behind_count: u32,
    pub ahead_count: u32,
    pub has_updates: bool,
    pub checked_at: u64,
    pub uncommitted_added: u32,
    pub uncommitted_removed: u32,
    pub branch_diff_added: u32,
    pub branch_diff_removed: u32,
    pub base_branch_ahead_count: u32,
    pub base_branch_behind_count: u32,
    pub worktree_ahead_count: u32,
    pub unpushed_count: u32,
}

impl GitService {
    pub fn branch_status(self, info: &ActiveWorktreeInfo) -> Result<GitBranchStatus, BackendError> {
        let root = Path::new(&info.worktree_path);
        let base = &info.base_branch;
        let _ = self.run(root, &["fetch", "origin", base]);
        let current = self.current_branch_with_detached_fallback(root)?;
        let origin_base = format!("origin/{base}");
        let behind_count = self.count_commits(root, "HEAD", &origin_base);
        let ahead_count = self.count_commits(root, &origin_base, "HEAD");
        let (uncommitted_added, uncommitted_removed) = self.uncommitted_stats(root);
        let (branch_diff_added, branch_diff_removed) = self.numstat(
            root,
            &["diff", "--numstat", &format!("{origin_base}...HEAD")],
        );
        let base_branch_ahead_count = self.count_commits(root, &origin_base, base);
        let base_branch_behind_count = self.count_commits(root, base, &origin_base);
        let worktree_ahead_count = self.count_commits(root, base, "HEAD");
        let unpushed_count = if current == *base {
            base_branch_ahead_count
        } else if let (Some(remote), Some(branch)) = (&info.pr_push_remote, &info.pr_push_branch) {
            let _ = self.run(root, &["fetch", remote, branch]);
            let push_ref = format!("{remote}/{branch}");
            if self.ref_exists(root, &push_ref) {
                self.count_commits(root, &push_ref, "HEAD")
            } else {
                worktree_ahead_count
            }
        } else if let Some(upstream) = self
            .text(root, &["rev-parse", "--abbrev-ref", "@{upstream}"])
            .ok()
            .filter(|upstream| !upstream.is_empty())
        {
            if let Some(remote) = upstream.split('/').next() {
                let _ = self.run(root, &["fetch", remote, &current]);
            }
            self.count_commits(root, &upstream, "HEAD")
        } else {
            let _ = self.run(root, &["fetch", "origin", &current]);
            let origin_current = format!("origin/{current}");
            if self.ref_exists(root, &origin_current) {
                self.count_commits(root, &origin_current, "HEAD")
            } else {
                worktree_ahead_count
            }
        };

        Ok(GitBranchStatus {
            worktree_id: info.worktree_id.clone(),
            current_branch: current,
            base_branch: base.clone(),
            behind_count,
            ahead_count,
            has_updates: behind_count > 0,
            checked_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            uncommitted_added,
            uncommitted_removed,
            branch_diff_added,
            branch_diff_removed,
            base_branch_ahead_count,
            base_branch_behind_count,
            worktree_ahead_count,
            unpushed_count,
        })
    }

    fn current_branch_with_detached_fallback(self, root: &Path) -> Result<String, BackendError> {
        self.text(root, &["symbolic-ref", "--short", "HEAD"])
            .or_else(|_| self.text(root, &["rev-parse", "--abbrev-ref", "HEAD"]))
    }

    fn ref_exists(self, root: &Path, reference: &str) -> bool {
        self.run(root, &["rev-parse", "--verify", "--quiet", reference])
            .is_ok_and(|output| output.status.success())
    }

    fn count_commits(self, root: &Path, from: &str, to: &str) -> u32 {
        self.text(root, &["rev-list", "--count", &format!("{from}..{to}")])
            .ok()
            .and_then(|count| count.parse().ok())
            .unwrap_or(0)
    }

    fn numstat(self, root: &Path, args: &[&str]) -> (u32, u32) {
        let Ok(output) = self.text(root, args) else {
            return (0, 0);
        };
        output.lines().fold((0, 0), |(added, removed), line| {
            let mut fields = line.split('\t');
            (
                added
                    + fields
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                removed
                    + fields
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
            )
        })
    }

    fn uncommitted_stats(self, root: &Path) -> (u32, u32) {
        let unstaged = self.numstat(root, &["diff", "--numstat"]);
        let staged = self.numstat(root, &["diff", "--cached", "--numstat"]);
        let untracked = self
            .text(root, &["ls-files", "--others", "--exclude-standard"])
            .unwrap_or_default()
            .lines()
            .filter(|path| !path.is_empty())
            .map(|path| {
                std::fs::read_to_string(root.join(path))
                    .map(|content| {
                        u32::try_from(content.lines().count())
                            .unwrap_or(u32::MAX)
                            .max(1)
                    })
                    .unwrap_or(1)
            })
            .sum::<u32>();
        (unstaged.0 + staged.0 + untracked, unstaged.1 + staged.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        assert!(Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }

    #[test]
    fn branch_status_counts_uncommitted_and_unpushed_changes() {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Jean Test"]);
        git(repo.path(), &["config", "user.email", "test@jean.local"]);
        std::fs::write(repo.path().join("tracked.txt"), "one\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "initial"]);
        git(repo.path(), &["checkout", "-b", "feature"]);
        std::fs::write(repo.path().join("new.txt"), "one\ntwo\n").unwrap();

        let status = GitService::default()
            .branch_status(&ActiveWorktreeInfo {
                worktree_id: "w1".to_string(),
                worktree_path: repo.path().display().to_string(),
                base_branch: "main".to_string(),
                pr_number: None,
                pr_url: None,
                pr_push_remote: None,
                pr_push_branch: None,
            })
            .unwrap();
        assert_eq!(status.current_branch, "feature");
        assert_eq!(status.uncommitted_added, 2);
        assert_eq!(status.unpushed_count, 0);
    }
}
