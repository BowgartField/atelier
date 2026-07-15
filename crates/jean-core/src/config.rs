use crate::{BackendError, BackendErrorCode};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Output};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortEntry {
    pub port: u16,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JeanConfig {
    #[serde(default)]
    pub scripts: JeanScripts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<PortEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RunScript {
    Single(String),
    Multiple(Vec<String>),
}

impl RunScript {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(script) => vec![script],
            Self::Multiple(scripts) => scripts,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JeanScripts {
    pub setup: Option<String>,
    pub teardown: Option<String>,
    pub run: Option<RunScript>,
}

pub fn read_jean_config(worktree_path: &str) -> Option<JeanConfig> {
    let contents = std::fs::read_to_string(Path::new(worktree_path).join("jean.json")).ok()?;
    serde_json::from_str(&contents).ok()
}

pub type ScriptRunner = fn(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &[(&str, &str)],
) -> Result<Output, BackendError>;

#[derive(Clone, Copy)]
pub struct ScriptService {
    runner: ScriptRunner,
}

impl Default for ScriptService {
    fn default() -> Self {
        Self::new(native_script_runner)
    }
}

impl ScriptService {
    pub fn new(runner: ScriptRunner) -> Self {
        Self { runner }
    }

    pub fn run_setup(
        self,
        worktree_path: &str,
        root_path: &str,
        branch: &str,
        script: &str,
    ) -> Result<String, BackendError> {
        self.run("setup", worktree_path, root_path, branch, script)
    }

    pub fn run_teardown(
        self,
        worktree_path: &str,
        root_path: &str,
        branch: &str,
        script: &str,
    ) -> Result<String, BackendError> {
        self.run("teardown", worktree_path, root_path, branch, script)
    }

    fn run(
        self,
        kind: &str,
        worktree_path: &str,
        root_path: &str,
        branch: &str,
        script: &str,
    ) -> Result<String, BackendError> {
        validate_script_env(worktree_path, root_path, branch)?;
        let (shell, login) = user_shell();
        let args = if login {
            vec![
                "-l".to_string(),
                "-i".to_string(),
                "-c".to_string(),
                script.to_string(),
            ]
        } else {
            vec!["-c".to_string(), script.to_string()]
        };
        let output = (self.runner)(
            &shell,
            &args,
            Path::new(worktree_path),
            &[
                ("JEAN_WORKSPACE_PATH", worktree_path),
                ("JEAN_ROOT_PATH", root_path),
                ("JEAN_BRANCH", branch),
            ],
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}").trim().to_string();
        if output.status.success() {
            Ok(combined)
        } else {
            Err(BackendError::new(
                BackendErrorCode::Io,
                format!("{kind} script failed:\n{combined}"),
            ))
        }
    }
}

fn validate_script_env(
    worktree_path: &str,
    root_path: &str,
    branch: &str,
) -> Result<(), BackendError> {
    for (name, value) in [
        ("JEAN_WORKSPACE_PATH", worktree_path),
        ("JEAN_ROOT_PATH", root_path),
        ("JEAN_BRANCH", branch),
    ] {
        if value.is_empty() {
            return Err(invalid(format!("{name} is empty — refusing to run script")));
        }
    }
    if !Path::new(worktree_path).is_absolute() {
        return Err(invalid(format!(
            "JEAN_WORKSPACE_PATH is not an absolute path: {worktree_path}"
        )));
    }
    if !Path::new(root_path).is_absolute() {
        return Err(invalid(format!(
            "JEAN_ROOT_PATH is not an absolute path: {root_path}"
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn user_shell() -> (String, bool) {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let login = ["bash", "zsh", "fish", "ksh", "tcsh"]
        .iter()
        .any(|name| shell.ends_with(name));
    (shell, login)
}

#[cfg(windows)]
fn user_shell() -> (String, bool) {
    ("powershell.exe".to_string(), false)
}

fn native_script_runner(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &[(&str, &str)],
) -> Result<Output, BackendError> {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().copied())
        .output()
        .map_err(BackendError::from)
}

fn invalid(message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorCode::InvalidArgument, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jean_config_supports_single_and_multiple_run_scripts() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("jean.json"),
            r#"{"scripts":{"setup":"echo setup","run":["bun dev","bun api"]},"ports":[{"port":3000,"label":"Web"}]}"#,
        )
        .unwrap();
        let config = read_jean_config(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(config.scripts.run.unwrap().into_vec().len(), 2);
        assert_eq!(config.ports.unwrap()[0].port, 3000);
    }

    #[cfg(unix)]
    #[test]
    fn script_service_validates_paths_and_exposes_jean_environment() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().to_str().unwrap();
        let output = ScriptService::default()
            .run_setup(path, path, "feature/shared", "printf '%s' \"$JEAN_BRANCH\"")
            .unwrap();
        assert_eq!(output, "feature/shared");
        assert!(ScriptService::default()
            .run_setup("relative", path, "main", "true")
            .is_err());
    }
}
