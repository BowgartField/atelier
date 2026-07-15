use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityClass {
    Core,
    AdapterBacked,
    DesktopOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandCapability {
    pub command: &'static str,
    pub class: CapabilityClass,
    pub available: bool,
}

macro_rules! core_capabilities {
    ($($command:literal),+ $(,)?) => {
        &[$(CommandCapability { command: $command, class: CapabilityClass::Core, available: true }),+]
    };
}

pub const HEADLESS_CAPABILITIES: &[CommandCapability] = core_capabilities![
    "get_server_platform",
    "get_server_status",
    "get_server_capabilities",
    "list_github_labels",
    "list_github_issues",
    "search_github_issues",
    "get_github_issue_by_number",
    "get_github_issue",
    "list_github_prs",
    "search_github_prs",
    "get_github_pr_by_number",
    "get_github_pr",
    "list_dependabot_alerts",
    "get_dependabot_alert",
    "list_repository_advisories",
    "get_repository_advisory",
    "load_security_alert_context",
    "list_loaded_security_contexts",
    "remove_security_context",
    "get_security_context_content",
    "load_advisory_context",
    "list_loaded_advisory_contexts",
    "remove_advisory_context",
    "get_advisory_context_content",
    "list_linear_teams",
    "list_linear_issues",
    "search_linear_issues",
    "get_linear_issue",
    "get_linear_issue_by_number",
    "load_linear_issue_context",
    "list_loaded_linear_issue_contexts",
    "get_linear_issue_context_contents",
    "remove_linear_issue_context",
    "load_issue_context",
    "list_loaded_issue_contexts",
    "remove_issue_context",
    "get_issue_context_content",
    "load_pr_context",
    "list_loaded_pr_contexts",
    "remove_pr_context",
    "get_pr_context_content",
    "load_preferences",
    "save_preferences",
    "patch_preferences",
    "load_ui_state",
    "save_ui_state",
    "list_projects",
    "add_project",
    "init_project",
    "init_git_in_folder",
    "clone_project",
    "remove_project",
    "list_worktrees",
    "get_worktree",
    "create_base_session",
    "close_base_session",
    "close_base_session_clean",
    "close_base_session_archive",
    "archive_worktree",
    "unarchive_worktree",
    "list_archived_worktrees",
    "create_worktree",
    "fork_session_to_worktree",
    "create_worktree_from_existing_branch",
    "checkout_pr",
    "import_worktree",
    "delete_worktree",
    "permanently_delete_worktree",
    "get_worktree_changes",
    "get_worktree_diff",
    "get_project_branches",
    "rename_worktree",
    "update_worktree_label",
    "update_worktree_labels",
    "set_worktree_last_opened",
    "update_worktree_cached_status",
    "update_project_settings",
    "reorder_projects",
    "reorder_worktrees",
    "has_uncommitted_changes",
    "get_git_diff",
    "get_commit_history",
    "get_commit_diff",
    "get_repo_branches",
    "git_pull",
    "git_push",
    "commit_changes",
    "revert_last_local_commit",
    "list_worktree_files",
    "get_git_remotes",
    "remove_git_remote",
    "revert_file",
    "git_stash",
    "git_stash_pop",
    "check_git_identity",
    "set_git_identity",
    "start_terminal",
    "terminal_write",
    "terminal_resize",
    "stop_terminal",
    "get_active_terminals",
    "has_active_terminal",
    "kill_all_terminals",
    "get_run_scripts",
    "get_ports",
    "get_sessions",
    "list_sessions_summary",
    "get_session_status",
    "get_session",
    "create_session",
    "rename_session",
    "close_session",
    "reorder_sessions",
    "set_active_session",
    "archive_session",
    "unarchive_session",
    "send_chat_message",
    "cancel_chat_message",
];

pub use crate::capabilities_generated::UNAVAILABLE_CAPABILITIES;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn capability_registry_has_no_duplicates_and_matches_dispatcher() {
        let dispatcher = include_str!("server.rs");
        let mut commands = HashSet::new();
        for capability in HEADLESS_CAPABILITIES {
            assert!(commands.insert(capability.command));
            assert!(dispatcher.contains(&format!("\"{}\" =>", capability.command)));
        }
        for capability in UNAVAILABLE_CAPABILITIES {
            assert!(commands.insert(capability.command));
            assert!(!capability.available);
        }
    }
}
