//! Configuration and path resolution for Command Code CLI.

use crate::platform::silent_command;
use std::path::PathBuf;
use tauri::AppHandle;

pub const CLI_BINARY_NAME: &str = "cmd";
pub const LEGACY_CLI_BINARY_NAME: &str = "command-code";

#[cfg(windows)]
pub const CLI_BINARY_CANDIDATES: &[&str] = &[
    "cmd.cmd",
    "cmd.ps1",
    "command-code.cmd",
    "command-code.ps1",
    "command-code.exe",
];

#[cfg(not(windows))]
pub const CLI_BINARY_CANDIDATES: &[&str] = &[CLI_BINARY_NAME, LEGACY_CLI_BINARY_NAME];

pub fn resolve_cli_binary(_app: &AppHandle) -> PathBuf {
    let which_cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };

    for binary_name in CLI_BINARY_CANDIDATES {
        if let Ok(output) = silent_command(which_cmd).arg(binary_name).output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path_str.is_empty() {
                    let path = PathBuf::from(&path_str);
                    if path.exists() {
                        return path;
                    }
                }
            }
        }
    }

    PathBuf::from(CLI_BINARY_NAME)
}
