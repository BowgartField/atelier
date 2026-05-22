//! Tauri commands for Command Code CLI management.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use tauri::AppHandle;

use super::config::{resolve_cli_binary, CLI_BINARY_CANDIDATES};
use crate::platform::silent_command;

const AUTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodeCliStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodeAuthStatus {
    pub authenticated: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodePathDetection {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub package_manager: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCodeInstallCommand {
    pub command: String,
    pub args: Vec<String>,
    pub description: String,
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek().is_some_and(|c| *c == '[') {
                let _ = chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_version(stdout: &[u8]) -> Option<String> {
    let version = strip_ansi(&String::from_utf8_lossy(stdout))
        .trim()
        .to_string();
    if version.is_empty() {
        None
    } else {
        Some(version.trim_start_matches('v').to_string())
    }
}

fn looks_authenticated(output: &str) -> bool {
    let lower = output.to_lowercase();
    if lower.contains("not authenticated")
        || lower.contains("not logged in")
        || lower.contains("login required")
    {
        return false;
    }
    lower.contains("authenticated")
        || lower.contains("logged in")
        || lower.contains("signed in")
        || lower.contains("user") && lower.contains('@')
}

enum TimedCommandResult {
    Output(Output),
    TimedOut,
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<TimedCommandResult, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to spawn command: {error}"))?;
    let start = std::time::Instant::now();

    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut handle) = child.stdout.take() {
                let _ = handle.read_to_end(&mut stdout);
            }
            if let Some(mut handle) = child.stderr.take() {
                let _ = handle.read_to_end(&mut stderr);
            }
            return Ok(TimedCommandResult::Output(Output {
                status,
                stdout,
                stderr,
            }));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(TimedCommandResult::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tauri::command]
pub async fn check_commandcode_cli_installed(
    app: AppHandle,
) -> Result<CommandCodeCliStatus, String> {
    let binary_path = resolve_cli_binary(&app);
    if !binary_path.exists() {
        return Ok(CommandCodeCliStatus {
            installed: false,
            version: None,
            path: None,
        });
    }
    let version = match silent_command(&binary_path).arg("--version").output() {
        Ok(output) if output.status.success() => parse_version(&output.stdout),
        Ok(output) => {
            log::warn!(
                "Command Code version command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            log::warn!("Failed to execute Command Code CLI: {error}");
            None
        }
    };
    Ok(CommandCodeCliStatus {
        installed: true,
        version,
        path: Some(binary_path.to_string_lossy().to_string()),
    })
}

#[tauri::command]
pub async fn check_commandcode_cli_auth(app: AppHandle) -> Result<CommandCodeAuthStatus, String> {
    let binary_path = resolve_cli_binary(&app);
    if !binary_path.exists() {
        return Ok(CommandCodeAuthStatus {
            authenticated: false,
            error: Some("Command Code CLI not found in PATH".to_string()),
            timed_out: false,
        });
    }

    for args in [["status"].as_slice(), ["whoami"].as_slice()] {
        let output = match run_command_with_timeout(
            {
                let mut command = silent_command(&binary_path);
                command.args(args);
                command
            },
            AUTH_CHECK_TIMEOUT,
        ) {
            Ok(TimedCommandResult::Output(output)) => output,
            Ok(TimedCommandResult::TimedOut) => {
                return Ok(CommandCodeAuthStatus {
                    authenticated: false,
                    error: Some(
                        "Command Code auth check timed out. Try again or run `cmd login`."
                            .to_string(),
                    ),
                    timed_out: true,
                })
            }
            Err(error) => {
                log::warn!(
                    "Failed to execute Command Code auth check {:?}: {error}",
                    args
                );
                continue;
            }
        };
        let combined = strip_ansi(&format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
        if looks_authenticated(&combined) {
            return Ok(CommandCodeAuthStatus {
                authenticated: true,
                error: None,
                timed_out: false,
            });
        }
        if !output.status.success() {
            let msg = combined.trim();
            return Ok(CommandCodeAuthStatus {
                authenticated: false,
                error: Some(if msg.is_empty() {
                    "Not authenticated. Run `cmd login`.".to_string()
                } else {
                    msg.to_string()
                }),
                timed_out: false,
            });
        }
    }
    Ok(CommandCodeAuthStatus {
        authenticated: false,
        error: Some("Not authenticated. Run `cmd login`.".to_string()),
        timed_out: false,
    })
}

#[tauri::command]
pub async fn detect_commandcode_in_path(
    _app: AppHandle,
) -> Result<CommandCodePathDetection, String> {
    let which_cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };
    let mut found_path = String::new();
    for binary_name in CLI_BINARY_CANDIDATES {
        found_path = match silent_command(which_cmd).arg(binary_name).output() {
            Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string(),
            _ => String::new(),
        };
        if !found_path.is_empty() {
            break;
        }
    }
    if found_path.is_empty() {
        return Ok(CommandCodePathDetection {
            found: false,
            path: None,
            version: None,
            package_manager: None,
        });
    }
    let version = silent_command(&found_path)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                parse_version(&o.stdout)
            } else {
                None
            }
        });
    let package_manager = if found_path.contains("/npm/") || found_path.contains("node_modules") {
        Some("npm".to_string())
    } else {
        None
    };
    Ok(CommandCodePathDetection {
        found: true,
        path: Some(found_path),
        version,
        package_manager,
    })
}

#[tauri::command]
pub async fn get_commandcode_install_command() -> Result<CommandCodeInstallCommand, String> {
    Ok(CommandCodeInstallCommand {
        command: "npm".to_string(),
        args: vec![
            "install".to_string(),
            "-g".to_string(),
            "command-code".to_string(),
        ],
        description: "Install Command Code globally with npm".to_string(),
    })
}
