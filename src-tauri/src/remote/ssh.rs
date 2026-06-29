use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tauri::AppHandle;
#[cfg(not(unix))]
use tauri::Manager;
#[cfg(unix)]
use uuid::Uuid;

use crate::platform::silent_command;

use super::keychain;
use super::types::{RemoteConnectionTest, RemoteServerAuth, RemoteServerConfig};

const CONNECT_TIMEOUT_SECONDS: &str = "10";
const CONTROL_PERSIST_SECONDS: &str = "600";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionKind {
    Command,
    Tunnel,
}

#[cfg(unix)]
fn ssh_runtime_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    static SSH_RUNTIME_DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = SSH_RUNTIME_DIR
        .get_or_init(|| PathBuf::from("/tmp").join(format!("jean-ssh-{}", Uuid::new_v4())))
        .clone();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create SSH runtime directory: {e}"))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("Failed to secure SSH runtime directory: {e}"))?;
    Ok(dir)
}

#[cfg(not(unix))]
fn ssh_runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {e}"))?
        .join("ssh");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create SSH runtime directory: {e}"))?;
    Ok(dir)
}

fn short_server_id(server_id: &str) -> String {
    server_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect()
}

fn control_path(app: &AppHandle, server_id: &str) -> Result<PathBuf, String> {
    Ok(ssh_runtime_dir(app)?.join(format!("{}.sock", short_server_id(server_id))))
}

fn ssh_config_path(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn common_ssh_args(
    app: &AppHandle,
    server: &RemoteServerConfig,
    kind: ConnectionKind,
    has_key_passphrase: bool,
) -> Result<Vec<String>, String> {
    #[cfg(unix)]
    let control_path = if kind == ConnectionKind::Command {
        Some(control_path(app, &server.id)?)
    } else {
        None
    };
    #[cfg(not(unix))]
    let control_path: Option<PathBuf> = None;

    Ok(common_ssh_args_for_path(
        server,
        kind,
        control_path.as_deref(),
        has_key_passphrase,
    ))
}

fn common_ssh_args_for_path(
    server: &RemoteServerConfig,
    kind: ConnectionKind,
    control_path: Option<&Path>,
    has_key_passphrase: bool,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        server.port.to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={CONNECT_TIMEOUT_SECONDS}"),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
    ];

    match &server.auth {
        RemoteServerAuth::SshKeyPath { path, .. } => {
            let batch_mode = if has_key_passphrase {
                "BatchMode=no"
            } else {
                "BatchMode=yes"
            };
            args.extend([
                "-i".to_string(),
                expand_tilde(path),
                "-o".to_string(),
                "IdentitiesOnly=yes".to_string(),
                "-o".to_string(),
                batch_mode.to_string(),
            ]);
        }
        RemoteServerAuth::Password { .. } => {
            args.extend([
                "-o".to_string(),
                "BatchMode=no".to_string(),
                "-o".to_string(),
                "PreferredAuthentications=password,keyboard-interactive".to_string(),
                "-o".to_string(),
                "PubkeyAuthentication=no".to_string(),
                "-o".to_string(),
                "NumberOfPasswordPrompts=1".to_string(),
            ]);
        }
    }

    if kind == ConnectionKind::Command {
        if let Some(control_path) = control_path {
            args.extend([
                "-o".to_string(),
                "ControlMaster=auto".to_string(),
                "-o".to_string(),
                format!("ControlPersist={CONTROL_PERSIST_SECONDS}"),
                "-o".to_string(),
                format!("ControlPath={}", ssh_config_path(control_path)),
            ]);
        }
    }

    args
}

fn expand_tilde(path: &str) -> String {
    if path == "~" {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from(path))
            .to_string_lossy()
            .to_string();
    }
    if let Some(remainder) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(remainder).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn target(server: &RemoteServerConfig) -> String {
    format!("{}@{}", server.username, server.host)
}

fn write_askpass_script(app: &AppHandle, server_id: &str) -> Result<PathBuf, String> {
    let runtime_dir = ssh_runtime_dir(app)?;
    #[cfg(windows)]
    let (path, contents) = (
        runtime_dir.join(format!("askpass-{}.cmd", short_server_id(server_id))),
        "@echo off\r\necho %JEAN_SSH_SECRET%\r\n",
    );
    #[cfg(not(windows))]
    let (path, contents) = (
        runtime_dir.join(format!("askpass-{}.sh", short_server_id(server_id))),
        "#!/bin/sh\nprintf '%s\\n' \"$JEAN_SSH_SECRET\"\n",
    );

    std::fs::write(&path, contents)
        .map_err(|e| format!("Failed to create SSH askpass helper: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to secure SSH askpass helper: {e}"))?;
    }

    Ok(path)
}

fn apply_auth_environment(
    command: &mut Command,
    app: &AppHandle,
    server_id: &str,
    secret: Option<&str>,
) -> Result<(), String> {
    if let Some(secret) = secret {
        let askpass = write_askpass_script(app, server_id)?;
        command
            .env("SSH_ASKPASS", askpass)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("JEAN_SSH_SECRET", secret)
            .env("DISPLAY", "jean-ssh:0");
    }
    Ok(())
}

fn auth_secret(server: &RemoteServerConfig) -> Result<Option<String>, String> {
    match &server.auth {
        RemoteServerAuth::SshKeyPath { passphrase, .. } => {
            if let Some(passphrase) = passphrase.as_deref().filter(|value| !value.is_empty()) {
                return Ok(Some(passphrase.to_string()));
            }
            keychain::load_passphrase(&server.id)
        }
        RemoteServerAuth::Password { password } => Ok(Some(password.clone())),
    }
}

pub fn build_ssh_command(
    app: &AppHandle,
    server: &RemoteServerConfig,
    remote_command: &str,
) -> Result<Command, String> {
    let secret = auth_secret(server)?;
    let mut command = silent_command("ssh");
    command.args(common_ssh_args(
        app,
        server,
        ConnectionKind::Command,
        secret.is_some(),
    )?);
    command.arg(target(server)).arg(remote_command);
    apply_auth_environment(&mut command, app, &server.id, secret.as_deref())?;
    Ok(command)
}

pub fn build_tunnel_command(
    app: &AppHandle,
    server: &RemoteServerConfig,
    local_port: u16,
) -> Result<Command, String> {
    let secret = auth_secret(server)?;
    let mut command = silent_command("ssh");
    command.args(common_ssh_args(
        app,
        server,
        ConnectionKind::Tunnel,
        secret.is_some(),
    )?);
    command.args([
        "-o",
        "ExitOnForwardFailure=yes",
        "-N",
        "-L",
        &tunnel_forward_arg(local_port, server.remote_port),
    ]);
    command.arg(target(server));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    apply_auth_environment(&mut command, app, &server.id, secret.as_deref())?;
    Ok(command)
}

fn tunnel_forward_arg(local_port: u16, remote_port: u16) -> String {
    format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}")
}

pub fn exec(
    app: &AppHandle,
    server: &RemoteServerConfig,
    remote_command: &str,
) -> Result<Output, String> {
    build_ssh_command(app, server, remote_command)?
        .output()
        .map_err(|e| format!("Failed to start system ssh: {e}"))
}

pub fn exec_checked(
    app: &AppHandle,
    server: &RemoteServerConfig,
    remote_command: &str,
) -> Result<String, String> {
    let output = exec(app, server, remote_command)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("Remote command failed with status {}", output.status)
    } else {
        format!("Remote command failed: {stderr}")
    })
}

pub fn scp_to(
    app: &AppHandle,
    server: &RemoteServerConfig,
    local_path: &Path,
    remote_path: &str,
) -> Result<(), String> {
    let secret = auth_secret(server)?;
    let mut command = silent_command("scp");
    let mut args = common_ssh_args(app, server, ConnectionKind::Command, secret.is_some())?;
    if let Some(port_index) = args.iter().position(|arg| arg == "-p") {
        args[port_index] = "-P".to_string();
    }
    command.args(args);
    command
        .arg(local_path)
        .arg(format!("{}:{remote_path}", target(server)));
    apply_auth_environment(&mut command, app, &server.id, secret.as_deref())?;
    let output = command
        .output()
        .map_err(|e| format!("Failed to start system scp: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("Failed to upload Jean artifact: {stderr}"))
    }
}

pub fn test_connection(
    app: &AppHandle,
    server: &RemoteServerConfig,
) -> Result<RemoteConnectionTest, String> {
    let output = exec(
        app,
        server,
        "printf '%s\\n' \"$(hostname)\" \"$(uname -s)\" \"$(uname -m)\"",
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Ok(RemoteConnectionTest {
            success: false,
            message: if stderr.is_empty() {
                format!("SSH exited with status {}", output.status)
            } else {
                stderr
            },
            hostname: None,
            os: None,
            architecture: None,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    Ok(RemoteConnectionTest {
        success: true,
        message: "SSH connection successful".to_string(),
        hostname: lines.next().map(str::to_string),
        os: lines.next().map(str::to_string),
        architecture: lines.next().map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::types::{RemoteServerAuth, RemoteServerStatus};

    fn key_server() -> RemoteServerConfig {
        RemoteServerConfig {
            id: "12345678-aaaa-bbbb-cccc-123456789012".to_string(),
            name: "Test".to_string(),
            host: "server.example.com".to_string(),
            port: 2222,
            username: "jean".to_string(),
            auth: RemoteServerAuth::SshKeyPath {
                path: "/tmp/id test".to_string(),
                passphrase: None,
            },
            default: false,
            remote_port: 3456,
            status: RemoteServerStatus::Disconnected,
            http_token: None,
            installed_version: None,
        }
    }

    #[test]
    fn command_args_keep_host_key_and_remote_command_as_distinct_arguments() {
        let mut args = common_ssh_args_for_path(
            &key_server(),
            ConnectionKind::Command,
            Some(Path::new("/tmp/jean.sock")),
            false,
        );
        args.push(target(&key_server()));
        args.push("printf '%s\\n' safe".to_string());

        assert!(args.windows(2).any(|pair| pair == ["-p", "2222"]));
        assert!(args.windows(2).any(|pair| pair == ["-i", "/tmp/id test"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-o", "ControlPath=\"/tmp/jean.sock\""]));
        assert!(args.iter().any(|arg| arg == "jean@server.example.com"));
        assert_eq!(args.last().unwrap(), "printf '%s\\n' safe");
    }

    #[test]
    fn control_path_is_quoted_for_openssh_config_parsing() {
        let args = common_ssh_args_for_path(
            &key_server(),
            ConnectionKind::Command,
            Some(Path::new("/tmp/Application Support/jean.sock")),
            false,
        );

        assert!(args
            .windows(2)
            .any(|pair| { pair == ["-o", "ControlPath=\"/tmp/Application Support/jean.sock\"",] }));
    }

    #[test]
    fn encrypted_key_enables_askpass_prompts() {
        let mut server = key_server();
        server.auth = RemoteServerAuth::SshKeyPath {
            path: "/tmp/id_encrypted".to_string(),
            passphrase: Some("secret".to_string()),
        };
        let args = common_ssh_args_for_path(&server, ConnectionKind::Command, None, true);

        assert!(args.windows(2).any(|pair| pair == ["-o", "BatchMode=no"]));
    }

    #[test]
    fn tunnel_args_bind_only_loopback() {
        assert_eq!(
            tunnel_forward_arg(45678, 3456),
            "127.0.0.1:45678:127.0.0.1:3456"
        );
    }

    #[test]
    fn tilde_expansion_preserves_non_tilde_paths() {
        assert_eq!(expand_tilde("/tmp/key"), "/tmp/key");
    }
}
