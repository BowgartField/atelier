use jean_core::{AppPaths, BackendContext, BackendError, EventSink};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Output;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub struct DesktopAppPaths {
    app: AppHandle,
}

impl DesktopAppPaths {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl AppPaths for DesktopAppPaths {
    fn data_dir(&self) -> Result<PathBuf, BackendError> {
        self.app
            .path()
            .app_data_dir()
            .map_err(|error| BackendError::new(jean_core::BackendErrorCode::Io, error.to_string()))
    }

    fn config_dir(&self) -> Result<PathBuf, BackendError> {
        self.app
            .path()
            .app_config_dir()
            .map_err(|error| BackendError::new(jean_core::BackendErrorCode::Io, error.to_string()))
    }

    fn cache_dir(&self) -> Result<PathBuf, BackendError> {
        self.app
            .path()
            .app_cache_dir()
            .map_err(|error| BackendError::new(jean_core::BackendErrorCode::Io, error.to_string()))
    }

    fn resource_dir(&self) -> Result<PathBuf, BackendError> {
        self.app
            .path()
            .resource_dir()
            .map_err(|error| BackendError::new(jean_core::BackendErrorCode::Io, error.to_string()))
    }
}

#[derive(Clone)]
pub struct DesktopEventSink {
    app: AppHandle,
}

impl DesktopEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for DesktopEventSink {
    fn emit_json(&self, event: &str, payload: Value) -> Result<(), BackendError> {
        self.app.emit(event, payload.clone()).map_err(|error| {
            BackendError::new(jean_core::BackendErrorCode::Internal, error.to_string())
        })?;
        if let Some(websocket) = self.app.try_state::<crate::http_server::WsBroadcaster>() {
            websocket.broadcast(event, &payload);
        }
        Ok(())
    }
}

pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    DesktopAppPaths::new(app.clone())
        .data_dir()
        .map_err(|error| error.to_string())
}

pub fn context(app: &AppHandle) -> Result<BackendContext, String> {
    app.try_state::<BackendContext>()
        .map(|context| context.inner().clone())
        .ok_or_else(|| "Backend context is not initialized".to_string())
}

fn desktop_git_runner(cwd: &std::path::Path, args: &[&str]) -> Result<Output, BackendError> {
    crate::platform::wsl_aware_command("git", Some(cwd))
        .args(args)
        .output()
        .map_err(|error| {
            BackendError::new(
                jean_core::BackendErrorCode::Io,
                format!("Failed to run git: {error}"),
            )
        })
}

fn desktop_script_runner(
    program: &str,
    args: &[String],
    cwd: &std::path::Path,
    env: &[(&str, &str)],
) -> Result<Output, BackendError> {
    let mut command = crate::platform::silent_command(program);
    command.args(args).current_dir(cwd);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().map_err(|error| {
        BackendError::new(
            jean_core::BackendErrorCode::Io,
            format!("Failed to run script: {error}"),
        )
    })
}

pub fn git_service() -> jean_core::GitService {
    jean_core::GitService::new(desktop_git_runner)
}

pub fn script_service() -> jean_core::ScriptService {
    jean_core::ScriptService::new(desktop_script_runner)
}

pub fn github_service(app: &AppHandle) -> jean_core::GitHubService {
    let app = app.clone();
    let runner: jean_core::GhRunner = Arc::new(move |project_path, args| {
        let gh = crate::gh_cli::config::resolve_gh_binary(&app);
        crate::platform::resolved_cli_command(&gh, Some(std::path::Path::new(project_path)))
            .args(args)
            .output()
            .map_err(|error| {
                BackendError::new(
                    jean_core::BackendErrorCode::Io,
                    format!("Failed to run gh: {error}"),
                )
            })
    });
    jean_core::GitHubService::new(runner)
}

pub fn linear_service(app: &AppHandle) -> Result<jean_core::LinearService, String> {
    Ok(jean_core::LinearService::new(context(app)?.persistence))
}

pub fn project_service(app: &AppHandle) -> Result<jean_core::ProjectService, String> {
    let context = context(app)?;
    let app_for_diff = app.clone();
    let pr_diff_loader: jean_core::PrDiffLoader = Arc::new(move |project_path, number| {
        let gh = crate::gh_cli::config::resolve_gh_binary(&app_for_diff);
        crate::projects::get_pr_diff(project_path, number, &gh)
            .map_err(|error| BackendError::new(jean_core::BackendErrorCode::Io, error))
    });
    let contexts = jean_core::ContextService::with_pr_diff_loader(
        context.persistence.clone(),
        git_service(),
        pr_diff_loader,
    );
    let github = github_service(app);
    let app_for_checkout = app.clone();
    let pr_checkout: jean_core::PrCheckout = Arc::new(move |worktree_path, number, branch| {
        let gh = crate::gh_cli::config::resolve_gh_binary(&app_for_checkout);
        crate::projects::git::gh_pr_checkout(worktree_path, number, branch, &gh)
            .map(|_| ())
            .map_err(|error| BackendError::new(jean_core::BackendErrorCode::Io, error))
    });
    Ok(jean_core::ProjectService::with_services(
        context.persistence,
        git_service(),
        script_service(),
        contexts,
        github,
        pr_checkout,
    ))
}
