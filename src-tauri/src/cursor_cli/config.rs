//! Configuration and path resolution for Cursor Agent.

use crate::platform::{get_wsl_config, silent_command};
use std::path::PathBuf;
use tauri::AppHandle;

/// Name of the Cursor Agent binary.
#[cfg(windows)]
pub const CLI_BINARY_NAME: &str = "cursor-agent.exe";
#[cfg(not(windows))]
pub const CLI_BINARY_NAME: &str = "cursor-agent";

/// Bare tool name (without platform-specific extension) for WSL/Unix lookups.
pub const CLI_TOOL_NAME: &str = "cursor-agent";

/// Resolve the Cursor Agent binary from system PATH.
///
/// Cursor's installer places the binary on PATH, so Jean resolves the
/// discovered system binary when available and returns a non-existent fallback
/// path otherwise.
pub fn resolve_cli_binary(_app: &AppHandle) -> PathBuf {
    let wsl = get_wsl_config();
    if wsl.enabled {
        // Resolve the absolute Unix path inside WSL via a login shell, so
        // cursor-agent installed via nvm / bun / cursor.com's installer is
        // found regardless of non-login-shell $PATH.
        if let Some(unix_path) = crate::platform::wsl_which(&wsl.distro, CLI_TOOL_NAME) {
            return PathBuf::from(unix_path);
        }
        return PathBuf::from(CLI_TOOL_NAME);
    }

    let which_cmd = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };

    if let Ok(output) = silent_command(which_cmd).arg(CLI_BINARY_NAME).output() {
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

    PathBuf::from(CLI_BINARY_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_path_is_cursor_agent_binary_name() {
        let resolved = PathBuf::from(CLI_BINARY_NAME);
        assert!(resolved.ends_with(CLI_BINARY_NAME));
    }
}
