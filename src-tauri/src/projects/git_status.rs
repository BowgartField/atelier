pub use jean_core::{ActiveWorktreeInfo, GitBranchStatus};

pub fn get_branch_status(info: &ActiveWorktreeInfo) -> Result<GitBranchStatus, String> {
    crate::backend_runtime::git_service()
        .branch_status(info)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_branch_status_serialization_keeps_the_desktop_contract() {
        let status = GitBranchStatus {
            worktree_id: "test-id".to_string(),
            current_branch: "feature/test".to_string(),
            base_branch: "main".to_string(),
            behind_count: 5,
            ahead_count: 3,
            has_updates: true,
            checked_at: 1_234_567_890,
            uncommitted_added: 10,
            uncommitted_removed: 5,
            branch_diff_added: 150,
            branch_diff_removed: 42,
            base_branch_ahead_count: 2,
            base_branch_behind_count: 0,
            worktree_ahead_count: 3,
            unpushed_count: 1,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"has_updates\":true"));
        assert!(json.contains("\"behind_count\":5"));
        assert!(json.contains("\"uncommitted_added\":10"));
        assert!(json.contains("\"branch_diff_added\":150"));
    }
}
