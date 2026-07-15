use crate::auth::validate_token;
use crate::{
    BackendContext, BackendError, BackendErrorCode, ChatService, GitService, ProjectService,
    SessionService, WsEvent,
};
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

#[derive(RustEmbed)]
#[folder = "../../dist"]
struct FrontendAssets;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub token_required: bool,
    pub allowed_origins: Vec<HeaderValue>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3456,
            token: String::new(),
            token_required: true,
            allowed_origins: Vec::new(),
        }
    }
}

impl ServerConfig {
    pub fn validate(&self) -> Result<IpAddr, BackendError> {
        let host = if self.host.eq_ignore_ascii_case("localhost") {
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        } else {
            self.host.parse::<IpAddr>().map_err(|error| {
                BackendError::new(
                    BackendErrorCode::InvalidArgument,
                    format!("Invalid bind host '{}': {error}", self.host),
                )
            })?
        };
        if !host.is_loopback() && !self.token_required {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Token authentication is required when binding outside loopback",
            ));
        }
        if self.token_required && self.token.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "A non-empty token is required",
            ));
        }
        Ok(host)
    }
}

pub fn parse_allowed_origins(raw: &str) -> Vec<HeaderValue> {
    raw.split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .filter_map(|origin| origin.parse().ok())
        .collect()
}

#[async_trait]
pub trait CommandDispatcher: Send + Sync {
    async fn dispatch(&self, command: &str, args: Value) -> Result<Value, BackendError>;
}

pub struct HeadlessDispatcher {
    context: BackendContext,
}

impl HeadlessDispatcher {
    pub fn new(context: BackendContext) -> Self {
        Self { context }
    }
}

#[async_trait]
impl CommandDispatcher for HeadlessDispatcher {
    async fn dispatch(&self, command: &str, args: Value) -> Result<Value, BackendError> {
        let projects = ProjectService::new(self.context.persistence.clone());
        let git = GitService::default();
        let sessions = SessionService::new(self.context.persistence.clone());
        let chat = ChatService::new(self.context.clone());
        match command {
            "get_server_platform" => Ok(Value::String(std::env::consts::OS.to_string())),
            "get_server_status" => Ok(serde_json::json!({
                "running": self.context.state.websocket.is_active(),
            })),
            "get_server_capabilities" => Ok(serde_json::json!({
                "commands": crate::HEADLESS_CAPABILITIES,
                "unavailable": crate::capabilities::UNAVAILABLE_CAPABILITIES,
            })),
            "load_preferences" => self.context.persistence.load_preferences(),
            "save_preferences" => {
                let preferences = required_field(&args, "preferences")?;
                self.context.persistence.save_preferences(preferences)?;
                self.context.events.emit_json(
                    "cache:invalidate",
                    serde_json::json!({"keys": ["preferences"]}),
                )?;
                Ok(Value::Null)
            }
            "patch_preferences" => {
                let patch = required_field(&args, "patch")?;
                let result = self.context.persistence.patch_preferences(patch)?;
                self.context.events.emit_json(
                    "cache:invalidate",
                    serde_json::json!({"keys": ["preferences"]}),
                )?;
                Ok(result)
            }
            "load_ui_state" => self.context.persistence.load_ui_state(),
            "save_ui_state" => {
                let ui_state = args
                    .get("uiState")
                    .or_else(|| args.get("ui_state"))
                    .ok_or_else(|| missing_field("uiState"))?;
                self.context.persistence.save_ui_state(ui_state)?;
                Ok(Value::Null)
            }
            "list_projects" => Ok(Value::Array(projects.list()?)),
            "add_project" => {
                let path = string_field(&args, "path", "path")?.to_string();
                let parent_id = optional_string_field(&args, "parentId", "parent_id")?;
                let project = projects.add(path, parent_id)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(project)
            }
            "init_project" => {
                let path = string_field(&args, "path", "path")?.to_string();
                let parent_id = optional_string_field(&args, "parentId", "parent_id")?;
                let project = projects.init(path, parent_id)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(project)
            }
            "init_git_in_folder" => {
                let path = string_field(&args, "path", "path")?.to_string();
                let repo = std::path::Path::new(&path);
                std::fs::create_dir_all(repo)?;
                let output = std::process::Command::new("git")
                    .current_dir(repo)
                    .arg("init")
                    .output()?;
                if !output.status.success() {
                    return Err(BackendError::new(
                        BackendErrorCode::Io,
                        String::from_utf8_lossy(&output.stderr).trim().to_string(),
                    ));
                }
                Ok(Value::String(path))
            }
            "clone_project" => {
                let url = string_field(&args, "url", "url")?;
                let path = string_field(&args, "path", "path")?.to_string();
                let parent_id = optional_string_field(&args, "parentId", "parent_id")?;
                let project = projects.clone_repository(url, path, parent_id)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(project)
            }
            "remove_project" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                projects.remove(project_id)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(Value::Null)
            }
            "list_worktrees" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                Ok(Value::Array(projects.list_worktrees(project_id)?))
            }
            "get_worktree" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                projects.get_worktree(worktree_id)
            }
            "create_base_session" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                let (worktree, _) = projects.create_base_session(project_id)?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(worktree)
            }
            "close_base_session" => close_base_session(
                &projects,
                &self.context,
                &args,
                crate::BaseSessionCloseMode::Preserve,
            ),
            "close_base_session_clean" => close_base_session(
                &projects,
                &self.context,
                &args,
                crate::BaseSessionCloseMode::Clean,
            ),
            "close_base_session_archive" => close_base_session(
                &projects,
                &self.context,
                &args,
                crate::BaseSessionCloseMode::Archive,
            ),
            "archive_worktree" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                projects.archive_worktree(worktree_id, self.context.events.as_ref())?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(Value::Null)
            }
            "unarchive_worktree" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let worktree =
                    projects.unarchive_worktree(worktree_id, self.context.events.as_ref())?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(worktree)
            }
            "list_archived_worktrees" => Ok(Value::Array(projects.list_archived_worktrees()?)),
            "create_worktree" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                let base_branch = optional_string_field(&args, "baseBranch", "base_branch")?;
                let custom_name = optional_string_field(&args, "customName", "custom_name")?;
                let origin = optional_string_field(&args, "origin", "origin")?;
                projects.create_worktree(
                    project_id,
                    base_branch.as_deref(),
                    custom_name.as_deref(),
                    origin.as_deref(),
                    self.context.events.as_ref(),
                )
            }
            "create_worktree_from_existing_branch" => {
                let project_id = string_field(&args, "projectId", "project_id")?.to_string();
                let branch_name = string_field(&args, "branchName", "branch_name")?.to_string();
                let contexts = crate::WorktreeContexts {
                    issue: optional_typed_field(&args, "issueContext", "issue_context")?,
                    pull_request: optional_typed_field(&args, "prContext", "pr_context")?,
                    security: optional_typed_field(&args, "securityContext", "security_context")?,
                    advisory: optional_typed_field(&args, "advisoryContext", "advisory_context")?,
                    linear: optional_typed_field(&args, "linearContext", "linear_context")?,
                };
                let auto_open_in_jean =
                    optional_bool_field(&args, "autoOpenInJean", "auto_open_in_jean")?
                        .unwrap_or(true);
                let (pending, task) = projects.prepare_existing_branch_worktree(
                    crate::ExistingBranchWorktreeInput {
                        project_id,
                        branch_name,
                        contexts,
                        auto_open_in_jean,
                    },
                    self.context.events.as_ref(),
                )?;
                let service = projects.clone();
                let events = self.context.events.clone();
                std::thread::spawn(move || {
                    service.run_existing_branch_worktree_task(task, events.as_ref());
                });
                Ok(pending)
            }
            "import_worktree" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                let path = string_field(&args, "path", "path")?;
                let worktree =
                    projects.import_worktree(project_id, path, self.context.events.as_ref())?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(worktree)
            }
            "delete_worktree" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                projects.delete_worktree(worktree_id, self.context.events.as_ref())?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(Value::Null)
            }
            "permanently_delete_worktree" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                projects.permanently_delete_worktree(worktree_id, self.context.events.as_ref())?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees", "sessions"])?;
                Ok(Value::Null)
            }
            "get_worktree_changes" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let max_files = optional_usize_field(&args, "maxFiles", "max_files")?
                    .unwrap_or(100)
                    .clamp(1, 500);
                projects.worktree_changes(worktree_id, max_files)
            }
            "get_worktree_diff" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let diff_type = optional_string_field(&args, "diffType", "diff_type")?
                    .unwrap_or_else(|| "uncommitted".to_string());
                let path = optional_string_field(&args, "path", "path")?;
                let max_bytes = optional_usize_field(&args, "maxBytes", "max_bytes")?
                    .unwrap_or(60_000)
                    .clamp(1, 200_000);
                projects.worktree_diff(worktree_id, &diff_type, path.as_deref(), max_bytes)
            }
            "get_project_branches" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                Ok(serde_json::to_value(projects.branches(project_id)?)?)
            }
            "rename_worktree" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let new_name = string_field(&args, "newName", "new_name")?;
                let worktree = projects.rename_worktree(worktree_id, new_name)?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(worktree)
            }
            "update_worktree_label" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let labels = args
                    .get("label")
                    .filter(|label| !label.is_null())
                    .cloned()
                    .into_iter()
                    .collect();
                projects.update_worktree_labels(worktree_id, labels)?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(Value::Null)
            }
            "update_worktree_labels" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let labels = required_field(&args, "labels")?
                    .as_array()
                    .cloned()
                    .ok_or_else(|| missing_field("labels"))?;
                projects.update_worktree_labels(worktree_id, labels)?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(Value::Null)
            }
            "set_worktree_last_opened" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                projects.set_worktree_last_opened(worktree_id)?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(Value::Null)
            }
            "update_worktree_cached_status" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                projects.update_worktree_cached_status(worktree_id, &args)?;
                emit_cache_invalidation(&self.context, &["projects", "worktrees"])?;
                Ok(Value::Null)
            }
            "update_project_settings" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                let project = projects.update(project_id, &args)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(project)
            }
            "reorder_projects" => {
                let project_ids = string_array_field(&args, "projectIds", "project_ids")?;
                projects.reorder(&project_ids)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(Value::Null)
            }
            "reorder_worktrees" => {
                let project_id = string_field(&args, "projectId", "project_id")?;
                let worktree_ids = string_array_field(&args, "worktreeIds", "worktree_ids")?;
                projects.reorder_worktrees(project_id, &worktree_ids)?;
                emit_cache_invalidation(&self.context, &["projects"])?;
                Ok(Value::Null)
            }
            "has_uncommitted_changes" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let worktree = projects.get_worktree(worktree_id)?;
                let path = worktree
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing_field("worktree.path"))?;
                Ok(Value::Bool(git.has_uncommitted_changes(path)?))
            }
            "get_git_diff" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                let diff_type = string_field(&args, "diffType", "diff_type")?;
                let base = optional_string_field(&args, "baseBranch", "base_branch")?;
                Ok(serde_json::to_value(git.diff(
                    path,
                    diff_type,
                    base.as_deref(),
                )?)?)
            }
            "get_commit_history" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                let branch = optional_string_field(&args, "branch", "branch")?;
                let limit = optional_u32_field(&args, "limit", "limit")?.unwrap_or(50);
                let skip = optional_u32_field(&args, "skip", "skip")?.unwrap_or(0);
                Ok(serde_json::to_value(git.commit_history(
                    path,
                    branch.as_deref(),
                    limit,
                    skip,
                )?)?)
            }
            "get_commit_diff" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                let sha = string_field(&args, "commitSha", "commit_sha")?;
                Ok(serde_json::to_value(git.commit_diff(path, sha)?)?)
            }
            "get_repo_branches" => {
                let path = string_field(&args, "repoPath", "repo_path")?;
                Ok(serde_json::to_value(git.branches(path)?)?)
            }
            "git_pull" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                let branch = string_field(&args, "baseBranch", "base_branch")?;
                let remote = optional_string_field(&args, "remote", "remote")?;
                Ok(Value::String(git.pull(path, branch, remote.as_deref())?))
            }
            "git_push" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                if optional_u32_field(&args, "prNumber", "pr_number")?.is_some() {
                    return Err(BackendError::new(
                        BackendErrorCode::Unsupported,
                        "PR-aware push requires the desktop GitHub adapter",
                    ));
                }
                let remote = optional_string_field(&args, "remote", "remote")?;
                Ok(serde_json::to_value(git.push(path, remote.as_deref())?)?)
            }
            "commit_changes" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let message = string_field(&args, "message", "message")?;
                let stage_all =
                    optional_bool_field(&args, "stageAll", "stage_all")?.unwrap_or(false);
                let worktree = projects.get_worktree(worktree_id)?;
                let path = worktree
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing_field("worktree.path"))?;
                Ok(Value::String(git.commit(path, message, stage_all)?))
            }
            "revert_last_local_commit" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                Ok(serde_json::to_value(git.revert_last_commit(path)?)?)
            }
            "list_worktree_files" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                let max = optional_usize_field(&args, "maxFiles", "max_files")?
                    .unwrap_or(5000)
                    .clamp(1, 20_000);
                Ok(serde_json::to_value(git.list_files(path, max)?)?)
            }
            "get_git_remotes" => {
                let path = string_field(&args, "repoPath", "repo_path")?;
                Ok(serde_json::to_value(git.remotes(path)?)?)
            }
            "remove_git_remote" => {
                let path = string_field(&args, "repoPath", "repo_path")?;
                let remote = string_field(&args, "remoteName", "remote_name")?;
                git.remove_remote(path, remote)?;
                Ok(Value::Null)
            }
            "revert_file" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                let file = string_field(&args, "filePath", "file_path")?;
                let status = string_field(&args, "fileStatus", "file_status")?;
                git.revert_file(path, file, status)?;
                Ok(Value::Null)
            }
            "git_stash" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                Ok(Value::String(git.stash(path)?))
            }
            "git_stash_pop" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                Ok(Value::String(git.stash_pop(path)?))
            }
            "check_git_identity" => Ok(serde_json::to_value(git.identity())?),
            "set_git_identity" => {
                let name = string_field(&args, "name", "name")?;
                let email = string_field(&args, "email", "email")?;
                git.set_identity(name, email)?;
                Ok(Value::Null)
            }
            "start_terminal" => {
                let terminal_id = string_field(&args, "terminalId", "terminal_id")?.to_string();
                let worktree_path =
                    string_field(&args, "worktreePath", "worktree_path")?.to_string();
                let cols = u16_field(&args, "cols")?;
                let rows = u16_field(&args, "rows")?;
                let command = optional_string_field(&args, "command", "command")?;
                let command_args =
                    optional_string_array_field(&args, "commandArgs", "command_args")?;
                self.context.state.terminals.start(
                    self.context.events.clone(),
                    terminal_id,
                    worktree_path,
                    cols,
                    rows,
                    command,
                    command_args,
                )?;
                Ok(Value::Null)
            }
            "terminal_write" => {
                let terminal_id = string_field(&args, "terminalId", "terminal_id")?;
                let data = string_field(&args, "data", "data")?;
                self.context.state.terminals.write(terminal_id, data)?;
                Ok(Value::Null)
            }
            "terminal_resize" => {
                let terminal_id = string_field(&args, "terminalId", "terminal_id")?;
                self.context.state.terminals.resize(
                    terminal_id,
                    u16_field(&args, "cols")?,
                    u16_field(&args, "rows")?,
                )?;
                Ok(Value::Null)
            }
            "stop_terminal" => {
                let terminal_id = string_field(&args, "terminalId", "terminal_id")?;
                Ok(Value::Bool(
                    self.context
                        .state
                        .terminals
                        .stop(self.context.events.as_ref(), terminal_id)?,
                ))
            }
            "get_active_terminals" => Ok(serde_json::to_value(
                self.context.state.terminals.active_ids(),
            )?),
            "has_active_terminal" => {
                let terminal_id = string_field(&args, "terminalId", "terminal_id")?;
                Ok(Value::Bool(self.context.state.terminals.has(terminal_id)))
            }
            "kill_all_terminals" => Ok(Value::from(self.context.state.terminals.kill_all())),
            "get_run_scripts" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                Ok(serde_json::to_value(crate::terminal::read_run_scripts(
                    path,
                ))?)
            }
            "get_ports" => {
                let path = string_field(&args, "worktreePath", "worktree_path")?;
                Ok(Value::Array(crate::terminal::read_ports(path)))
            }
            "list_sessions_summary" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let include_archived =
                    optional_bool_field(&args, "includeArchived", "include_archived")?
                        .unwrap_or(false);
                sessions.summaries(worktree_id, include_archived)
            }
            "get_sessions" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let include_archived =
                    optional_bool_field(&args, "includeArchived", "include_archived")?
                        .unwrap_or(false);
                sessions.list(worktree_id, include_archived)
            }
            "get_session_status" => {
                let session_id = string_field(&args, "sessionId", "session_id")?;
                sessions.status(
                    session_id,
                    self.context.state.chat_runs.contains(session_id),
                )
            }
            "get_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let session_id = string_field(&args, "sessionId", "session_id")?;
                sessions.get(worktree_id, session_id)
            }
            "create_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let name = optional_string_field(&args, "name", "name")?;
                let backend = optional_string_field(&args, "backend", "backend")?;
                let primary_surface =
                    optional_string_field(&args, "primarySurface", "primary_surface")?;
                let terminal_command =
                    optional_string_field(&args, "terminalCommand", "terminal_command")?;
                let terminal_command_args = optional_string_array_field(
                    &args,
                    "terminalCommandArgs",
                    "terminal_command_args",
                )?;
                let terminal_label =
                    optional_string_field(&args, "terminalLabel", "terminal_label")?;
                let native_session_id =
                    optional_string_field(&args, "nativeSessionId", "native_session_id")?;
                let session = sessions.create(
                    worktree_id,
                    name.as_deref(),
                    backend.as_deref(),
                    primary_surface.as_deref(),
                    terminal_command.as_deref(),
                    terminal_command_args.as_deref(),
                    terminal_label.as_deref(),
                    native_session_id.as_deref(),
                )?;
                emit_cache_invalidation(&self.context, &["sessions"])?;
                Ok(session)
            }
            "rename_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let session_id = string_field(&args, "sessionId", "session_id")?;
                let new_name = string_field(&args, "newName", "new_name")?;
                sessions.rename(worktree_id, session_id, new_name)?;
                emit_cache_invalidation(&self.context, &["sessions"])?;
                Ok(Value::Null)
            }
            "close_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let session_id = string_field(&args, "sessionId", "session_id")?;
                let active = sessions.close(worktree_id, session_id)?;
                emit_cache_invalidation(&self.context, &["sessions"])?;
                Ok(serde_json::to_value(active)?)
            }
            "reorder_sessions" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let ids = string_array_field(&args, "sessionIds", "session_ids")?;
                sessions.reorder(worktree_id, &ids)?;
                emit_cache_invalidation(&self.context, &["sessions"])?;
                Ok(Value::Null)
            }
            "set_active_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let session_id = string_field(&args, "sessionId", "session_id")?;
                sessions.set_active(worktree_id, session_id)?;
                emit_cache_invalidation(&self.context, &["sessions"])?;
                Ok(Value::Null)
            }
            "archive_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let session_id = string_field(&args, "sessionId", "session_id")?;
                sessions.archive(worktree_id, session_id, true)
            }
            "unarchive_session" => {
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let session_id = string_field(&args, "sessionId", "session_id")?;
                sessions.archive(worktree_id, session_id, false)
            }
            "send_chat_message" => {
                let session_id = string_field(&args, "sessionId", "session_id")?;
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                let worktree_path = string_field(&args, "worktreePath", "worktree_path")?;
                let message = string_field(&args, "message", "message")?;
                let backend = optional_string_field(&args, "backend", "backend")?;
                let model = optional_string_field(&args, "model", "model")?;
                let execution_mode =
                    optional_string_field(&args, "executionMode", "execution_mode")?;
                let thinking_level =
                    optional_string_field(&args, "thinkingLevel", "thinking_level")?;
                let effort_level = optional_string_field(&args, "effortLevel", "effort_level")?;
                chat.send(
                    session_id,
                    worktree_id,
                    worktree_path,
                    message,
                    backend.as_deref(),
                    model.as_deref(),
                    execution_mode.as_deref(),
                    thinking_level.as_deref(),
                    effort_level.as_deref(),
                )
                .await
            }
            "cancel_chat_message" => {
                let session_id = string_field(&args, "sessionId", "session_id")?;
                let worktree_id = string_field(&args, "worktreeId", "worktree_id")?;
                chat.cancel(session_id, worktree_id)?;
                Ok(Value::Null)
            }
            _ => Err(BackendError::unsupported(command)),
        }
    }
}

fn emit_cache_invalidation(context: &BackendContext, keys: &[&str]) -> Result<(), BackendError> {
    context
        .events
        .emit_json("cache:invalidate", serde_json::json!({"keys": keys}))
}

fn close_base_session(
    projects: &ProjectService,
    context: &BackendContext,
    args: &Value,
    mode: crate::BaseSessionCloseMode,
) -> Result<Value, BackendError> {
    let worktree_id = string_field(args, "worktreeId", "worktree_id")?;
    projects.close_base_session(worktree_id, mode, context.events.as_ref())?;
    emit_cache_invalidation(context, &["projects", "worktrees", "sessions"])?;
    Ok(Value::Null)
}

fn required_field<'a>(args: &'a Value, field: &str) -> Result<&'a Value, BackendError> {
    args.get(field).ok_or_else(|| missing_field(field))
}

fn string_field<'a>(
    args: &'a Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<&'a str, BackendError> {
    args.get(camel_case)
        .or_else(|| args.get(snake_case))
        .and_then(Value::as_str)
        .ok_or_else(|| missing_field(camel_case))
}

fn optional_string_field(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Option<String>, BackendError> {
    match args.get(camel_case).or_else(|| args.get(snake_case)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(missing_field(camel_case)),
    }
}

fn string_array_field(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Vec<String>, BackendError> {
    args.get(camel_case)
        .or_else(|| args.get(snake_case))
        .and_then(Value::as_array)
        .ok_or_else(|| missing_field(camel_case))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| missing_field(camel_case))
        })
        .collect()
}

fn optional_string_array_field(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Option<Vec<String>>, BackendError> {
    match args.get(camel_case).or_else(|| args.get(snake_case)) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => string_array_field(args, camel_case, snake_case).map(Some),
    }
}

fn u16_field(args: &Value, field: &str) -> Result<u16, BackendError> {
    args.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| missing_field(field))
}

fn optional_usize_field(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Option<usize>, BackendError> {
    match args.get(camel_case).or_else(|| args.get(snake_case)) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| missing_field(camel_case)),
    }
}

fn optional_u32_field(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Option<u32>, BackendError> {
    match args.get(camel_case).or_else(|| args.get(snake_case)) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| missing_field(camel_case)),
    }
}

fn optional_bool_field(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Option<bool>, BackendError> {
    match args.get(camel_case).or_else(|| args.get(snake_case)) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| missing_field(camel_case)),
    }
}

fn optional_typed_field<T: serde::de::DeserializeOwned>(
    args: &Value,
    camel_case: &str,
    snake_case: &str,
) -> Result<Option<T>, BackendError> {
    match args.get(camel_case).or_else(|| args.get(snake_case)) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| missing_field(camel_case)),
    }
}

fn missing_field(field: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidArgument,
        format!("Missing or invalid field '{field}'"),
    )
}

#[derive(Clone)]
struct AppState {
    context: BackendContext,
    config: Arc<ServerConfig>,
    dispatcher: Arc<dyn CommandDispatcher>,
}

#[derive(Default, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum WsClientMessage {
    #[serde(rename = "invoke")]
    Invoke {
        id: String,
        command: String,
        #[serde(default)]
        args: Value,
    },
    #[serde(rename = "replay")]
    Replay { session_id: String, last_seq: u64 },
    #[serde(rename = "terminal_replay")]
    TerminalReplay { terminal_id: String, last_seq: u64 },
}

#[derive(Deserialize)]
struct LegacyInvoke {
    id: String,
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Serialize)]
struct InvokeResponse {
    #[serde(rename = "type")]
    kind: &'static str,
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<crate::BackendErrorCode>,
}

pub struct ServerHandle {
    pub address: SocketAddr,
    context: BackendContext,
    task: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl ServerHandle {
    pub async fn shutdown(self) -> Result<(), BackendError> {
        self.context.shutdown();
        self.task
            .await
            .map_err(|error| BackendError::new(BackendErrorCode::Internal, error.to_string()))??;
        Ok(())
    }
}

pub fn router(
    context: BackendContext,
    config: ServerConfig,
    dispatcher: Arc<dyn CommandDispatcher>,
) -> Router {
    let allow_any_origin = config
        .allowed_origins
        .iter()
        .any(|origin| origin.as_bytes() == b"*");
    let cors = if allow_any_origin {
        CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(AllowOrigin::any())
    } else if config.allowed_origins.is_empty() {
        CorsLayer::new().allow_methods(Any).allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(AllowOrigin::list(config.allowed_origins.clone()))
    };
    let state = AppState {
        context,
        config: Arc::new(config),
        dispatcher,
    };
    Router::new()
        .route("/healthz", get(health_handler))
        .route("/readyz", get(ready_handler))
        .route("/api/auth", get(auth_handler))
        .route("/api/init", get(init_handler))
        .route("/api/version", get(version_handler))
        .route("/ws", get(ws_handler))
        .fallback(get(static_handler))
        .layer(CompressionLayer::new().br(true).gzip(true))
        .layer(cors)
        .with_state(state)
}

pub async fn spawn(
    context: BackendContext,
    config: ServerConfig,
    dispatcher: Arc<dyn CommandDispatcher>,
) -> Result<ServerHandle, BackendError> {
    let bind_ip = config.validate()?;
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind_ip, config.port)).await?;
    let address = listener.local_addr()?;
    let app = router(context.clone(), config, dispatcher);
    context.state.websocket.set_active(true);
    let shutdown = context.state.shutdown.clone();
    let broadcaster = context.state.websocket.clone();
    let task = tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await;
        broadcaster.set_active(false);
        result
    });
    Ok(ServerHandle {
        address,
        context,
        task,
    })
}

async fn health_handler() -> Json<Value> {
    Json(serde_json::json!({"ok": true}))
}

async fn ready_handler(State(state): State<AppState>) -> Response {
    let ready = state.context.state.websocket.is_active();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({
            "ok": ready,
            "http": true,
            "websocket_broadcaster": ready,
        })),
    )
        .into_response()
}

fn authorized(query_token: Option<&str>, headers: &HeaderMap, config: &ServerConfig) -> bool {
    if !config.token_required {
        return true;
    }
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    query_token
        .or(bearer)
        .is_some_and(|provided| validate_token(provided, &config.token))
}

async fn auth_handler(
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if !authorized(query.token.as_deref(), &headers, &state.config) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"ok": false, "error": "Invalid token"})),
        )
            .into_response();
    }
    Json(serde_json::json!({
        "ok": true,
        "token_required": state.config.token_required,
        "webBuildId": env!("CARGO_PKG_VERSION"),
        "appVersion": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn version_handler(
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if !authorized(query.token.as_deref(), &headers, &state.config) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({
        "webBuildId": env!("CARGO_PKG_VERSION"),
        "appVersion": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn init_handler(
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if !authorized(query.token.as_deref(), &headers, &state.config) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let data_dir = state
        .context
        .paths
        .data_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let projects = state
        .context
        .persistence
        .load_projects()
        .unwrap_or_default();
    let preferences = state
        .context
        .persistence
        .load_preferences()
        .unwrap_or_else(|_| serde_json::json!({}));
    let ui_state = state
        .context
        .persistence
        .load_ui_state()
        .unwrap_or_else(|_| serde_json::json!({}));
    let mut worktrees_by_project = serde_json::Map::new();
    let mut sessions_by_worktree = serde_json::Map::new();
    let session_service = SessionService::new(state.context.persistence.clone());
    for worktree in &projects.worktrees {
        if worktree
            .get("archived_at")
            .is_some_and(|value| !value.is_null())
        {
            continue;
        }
        if let Some(project_id) = worktree.get("project_id").and_then(Value::as_str) {
            worktrees_by_project
                .entry(project_id.to_string())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("worktree groups are arrays")
                .push(worktree.clone());
        }
        if let Some(worktree_id) = worktree.get("id").and_then(Value::as_str) {
            if let Ok(sessions) = session_service.list(worktree_id, false) {
                sessions_by_worktree.insert(worktree_id.to_string(), sessions);
            }
        }
    }
    let running_sessions = state.context.state.chat_runs.active_ids();
    Json(serde_json::json!({
        "projects": projects.projects,
        "worktreesByProject": worktrees_by_project,
        "sessionsByWorktree": sessions_by_worktree,
        "activeSessions": {},
        "activeSessionWorktreeIds": {},
        "runningSessions": running_sessions,
        "replayEvents": [],
        "preferences": preferences,
        "uiState": ui_state,
        "appDataDir": data_dir,
        "serverPlatform": std::env::consts::OS,
        "webBuildId": env!("CARGO_PKG_VERSION"),
        "appVersion": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if !authorized(query.token.as_deref(), &headers, &state.config) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let events = state.context.state.websocket.subscribe();
    ws.on_upgrade(move |socket| handle_socket(socket, state, events))
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    mut events: broadcast::Receiver<WsEvent>,
) {
    let (mut sender, mut receiver) = socket.split();
    let (responses_tx, mut responses_rx) = mpsc::unbounded_channel::<String>();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.tick().await;
    let mut last_inbound = Instant::now();

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_inbound.elapsed() > Duration::from_secs(45) {
                    break;
                }
                if sender.send(Message::Ping(Vec::new().into())).await.is_err()
                    || sender.send(Message::Text(r#"{"type":"heartbeat"}"#.into())).await.is_err()
                {
                    break;
                }
            }
            incoming = receiver.next() => {
                last_inbound = Instant::now();
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        handle_client_message(&state, &responses_tx, &text).await;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            response = responses_rx.recv() => {
                match response {
                    Some(response) => {
                        if sender.send(Message::Text(response.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) if sender.send(Message::Text(event.json.to_string().into())).await.is_err() => break,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    _ => {}
                }
            }
        }
    }
}

async fn handle_client_message(
    state: &AppState,
    responses: &mpsc::UnboundedSender<String>,
    text: &str,
) {
    let parsed = serde_json::from_str::<WsClientMessage>(text).or_else(|_| {
        serde_json::from_str::<LegacyInvoke>(text).map(|legacy| WsClientMessage::Invoke {
            id: legacy.id,
            command: legacy.command,
            args: legacy.args,
        })
    });
    match parsed {
        Ok(WsClientMessage::Invoke { id, command, args }) => {
            let dispatcher = state.dispatcher.clone();
            let responses = responses.clone();
            tokio::spawn(async move {
                let response = match dispatcher.dispatch(&command, args).await {
                    Ok(data) => InvokeResponse {
                        kind: "response",
                        id,
                        data: Some(data),
                        error: None,
                        error_code: None,
                    },
                    Err(error) => InvokeResponse {
                        kind: "error",
                        id,
                        data: None,
                        error: Some(error.message),
                        error_code: Some(error.code),
                    },
                };
                if let Ok(json) = serde_json::to_string(&response) {
                    let _ = responses.send(json);
                }
            });
        }
        Ok(WsClientMessage::Replay {
            session_id,
            last_seq,
        }) => {
            for event in state
                .context
                .state
                .websocket
                .replay_events(&session_id, last_seq)
            {
                let _ = responses.send(event.json.to_string());
            }
        }
        Ok(WsClientMessage::TerminalReplay {
            terminal_id,
            last_seq,
        }) => {
            for event in state
                .context
                .state
                .websocket
                .replay_terminal_events(&terminal_id, last_seq)
            {
                let _ = responses.send(event.json.to_string());
            }
        }
        Err(error) => {
            log::warn!("Ignoring invalid WebSocket message: {error}");
        }
    }
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let requested = if path.is_empty() { "index.html" } else { path };
    let exact_asset = FrontendAssets::get(requested);
    let (asset, served_path) = match exact_asset {
        Some(asset) => (Some(asset), requested),
        None => (FrontendAssets::get("index.html"), "index.html"),
    };
    match asset {
        Some(asset) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type(served_path))],
            Body::from(asset.data.into_owned()),
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Frontend assets not found. Run `bun run build` before building jean-server.",
        )
            .into_response(),
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BackendState, ResolvedAppPaths, ServerEventSink, WsBroadcaster};
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_context() -> BackendContext {
        let broadcaster = Arc::new(WsBroadcaster::new());
        broadcaster.set_active(true);
        let data_dir = tempfile::tempdir().unwrap().keep();
        BackendContext::new(
            Arc::new(ResolvedAppPaths::new(
                data_dir.clone(),
                data_dir.join("config"),
                data_dir.join("cache"),
                data_dir.join("resources"),
            )),
            Arc::new(ServerEventSink::new(broadcaster.clone())),
            Arc::new(BackendState::new(broadcaster)),
        )
    }

    fn test_router() -> Router {
        let context = test_context();
        router(
            context.clone(),
            ServerConfig {
                token: "test-token".to_string(),
                ..ServerConfig::default()
            },
            Arc::new(HeadlessDispatcher::new(context)),
        )
    }

    #[test]
    fn wildcard_bind_requires_authentication() {
        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            token_required: false,
            ..ServerConfig::default()
        };
        assert_eq!(
            config.validate().unwrap_err().code,
            BackendErrorCode::InvalidArgument
        );
    }

    #[test]
    fn localhost_resolves_to_loopback() {
        let config = ServerConfig {
            host: "localhost".to_string(),
            token: "token".to_string(),
            ..ServerConfig::default()
        };
        assert!(config.validate().unwrap().is_loopback());
    }

    #[tokio::test]
    async fn health_and_ready_are_available_without_tauri() {
        for endpoint in ["/healthz", "/readyz"] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .uri(endpoint)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn auth_accepts_query_and_bearer_tokens() {
        let query_response = test_router()
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(query_response.status(), StatusCode::OK);

        let bearer_response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/auth")
                    .header(header::AUTHORIZATION, "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bearer_response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn init_requires_auth_and_uses_compatible_data_dir() {
        let unauthorized = test_router()
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/init")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/init?token=test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert!(value["appDataDir"]
            .as_str()
            .is_some_and(|path| !path.is_empty()));
        assert_eq!(value["projects"], serde_json::json!([]));
        assert_eq!(value["preferences"], serde_json::json!({}));
        assert_eq!(value["uiState"], serde_json::json!({}));
    }

    #[tokio::test]
    async fn headless_dispatcher_persists_preferences_and_filters_worktrees() {
        let context = test_context();
        context
            .persistence
            .save_projects(&crate::ProjectsSnapshot {
                projects: vec![serde_json::json!({"id": "p1", "name": "One"})],
                worktrees: vec![
                    serde_json::json!({"id": "w1", "project_id": "p1"}),
                    serde_json::json!({"id": "w2", "project_id": "p2"}),
                ],
                extra: serde_json::Map::new(),
            })
            .unwrap();
        let dispatcher = HeadlessDispatcher::new(context);

        dispatcher
            .dispatch(
                "save_preferences",
                serde_json::json!({"preferences": {"theme": "dark"}}),
            )
            .await
            .unwrap();
        assert_eq!(
            dispatcher
                .dispatch("load_preferences", serde_json::json!({}))
                .await
                .unwrap()["theme"],
            "dark"
        );

        let worktrees = dispatcher
            .dispatch("list_worktrees", serde_json::json!({"projectId": "p1"}))
            .await
            .unwrap();
        assert_eq!(worktrees.as_array().unwrap().len(), 1);
        assert_eq!(worktrees[0]["id"], "w1");
    }

    #[tokio::test]
    async fn headless_dispatcher_supports_base_and_archived_worktree_lifecycle() {
        let context = test_context();
        let repo = tempfile::tempdir().unwrap();
        context
            .persistence
            .save_projects(&crate::ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"p1","name":"Repo","path":repo.path(),"default_branch":"main"
                })],
                worktrees: vec![serde_json::json!({
                    "id":"w1","project_id":"p1","path":repo.path(),"session_type":"worktree"
                })],
                extra: serde_json::Map::new(),
            })
            .unwrap();
        let dispatcher = HeadlessDispatcher::new(context);

        let base = dispatcher
            .dispatch("create_base_session", serde_json::json!({"projectId":"p1"}))
            .await
            .unwrap();
        assert_eq!(base["session_type"], "base");
        dispatcher
            .dispatch("archive_worktree", serde_json::json!({"worktreeId":"w1"}))
            .await
            .unwrap();
        assert_eq!(
            dispatcher
                .dispatch("list_archived_worktrees", serde_json::json!({}))
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        dispatcher
            .dispatch("unarchive_worktree", serde_json::json!({"worktreeId":"w1"}))
            .await
            .unwrap();
        dispatcher
            .dispatch(
                "close_base_session_clean",
                serde_json::json!({"worktreeId":base["id"]}),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn headless_dispatcher_imports_and_permanently_deletes_worktrees() {
        let context = test_context();
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let imported_path = root.path().join("imported");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.name", "Jean Server Tests"],
            vec!["config", "user.email", "jean-server@example.test"],
        ] {
            assert!(std::process::Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        std::fs::write(repo.join("README.md"), "server").unwrap();
        for args in [vec!["add", "README.md"], vec!["commit", "-m", "initial"]] {
            assert!(std::process::Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        }
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args([
                "worktree",
                "add",
                "-b",
                "feature/server-import",
                imported_path.to_str().unwrap(),
                "main",
            ])
            .output()
            .unwrap()
            .status
            .success());
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["branch", "feature/server-core"])
            .output()
            .unwrap()
            .status
            .success());
        context
            .persistence
            .save_projects(&crate::ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"p1","name":"Repo","path":repo,"default_branch":"main",
                    "worktrees_dir":root.path().join("worktrees")
                })],
                worktrees: vec![],
                extra: serde_json::Map::new(),
            })
            .unwrap();
        let dispatcher = HeadlessDispatcher::new(context);
        let pending = dispatcher
            .dispatch(
                "create_worktree_from_existing_branch",
                serde_json::json!({
                    "projectId":"p1",
                    "branchName":"feature/server-core",
                    "issueContext":{
                        "number":99,"title":"Server core","body":null,"comments":[]
                    },
                    "autoOpenInJean":false
                }),
            )
            .await
            .unwrap();
        let pending_id = pending["id"].as_str().unwrap();
        let mut completed = None;
        for _ in 0..100 {
            if let Ok(worktree) = dispatcher
                .dispatch("get_worktree", serde_json::json!({"worktreeId":pending_id}))
                .await
            {
                completed = Some(worktree);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let completed = completed.expect("background core worktree creation completes");
        assert_eq!(completed["issue_number"], 99);
        assert!(std::path::Path::new(completed["path"].as_str().unwrap()).exists());
        let imported = dispatcher
            .dispatch(
                "import_worktree",
                serde_json::json!({"projectId":"p1","path":imported_path}),
            )
            .await
            .unwrap();
        assert_eq!(imported["branch"], "feature/server-import");
        let worktree_id = imported["id"].as_str().unwrap();
        dispatcher
            .dispatch(
                "archive_worktree",
                serde_json::json!({"worktreeId":worktree_id}),
            )
            .await
            .unwrap();
        dispatcher
            .dispatch(
                "permanently_delete_worktree",
                serde_json::json!({"worktreeId":worktree_id}),
            )
            .await
            .unwrap();
        assert!(!imported_path.exists());
        assert!(dispatcher
            .dispatch(
                "get_worktree",
                serde_json::json!({"worktreeId":worktree_id}),
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn real_websocket_dispatches_commands_and_returns_typed_errors() {
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        let context = test_context();
        let handle = spawn(
            context.clone(),
            ServerConfig {
                port: 0,
                token: "test-token".to_string(),
                ..ServerConfig::default()
            },
            Arc::new(HeadlessDispatcher::new(context)),
        )
        .await
        .unwrap();
        let url = format!("ws://{}/ws?token=test-token", handle.address);
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();

        socket
            .send(ClientMessage::Text(
                serde_json::json!({
                    "type": "invoke",
                    "id": "platform",
                    "command": "get_server_platform",
                    "args": {},
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let response: Value = serde_json::from_str(
            socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        assert_eq!(response["type"], "response");
        assert_eq!(response["id"], "platform");
        assert_eq!(response["data"], std::env::consts::OS);

        socket
            .send(ClientMessage::Text(
                serde_json::json!({
                    "type": "invoke",
                    "id": "unsupported",
                    "command": "open_main_window",
                    "args": {},
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let response: Value = serde_json::from_str(
            socket
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        assert_eq!(response["type"], "error");
        assert_eq!(response["error_code"], "unsupported");

        socket.close(None).await.unwrap();
        handle.shutdown().await.unwrap();
    }
}
