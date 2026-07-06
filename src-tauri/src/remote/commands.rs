use tauri::AppHandle;
use uuid::Uuid;

use super::keychain;
use super::provision;
use super::ssh;
use super::tunnel;
use super::types::{
    ProvisionResult, RemoteConnection, RemoteConnectionTest, RemoteJeanVersionInfo,
    RemoteServerAuth, RemoteServerConfig, RemoteServerInput, RemoteServerStatus,
    RemoteServerStatusInfo,
};

fn find_server(
    servers: &[RemoteServerConfig],
    server_id: &str,
) -> Result<RemoteServerConfig, String> {
    servers
        .iter()
        .find(|server| server.id == server_id)
        .cloned()
        .ok_or_else(|| format!("Remote server not found: {server_id}"))
}

fn normalize_default(servers: &mut [RemoteServerConfig], preferred_id: Option<&str>) {
    if let Some(preferred_id) = preferred_id {
        for server in servers {
            server.default = server.id == preferred_id;
        }
        return;
    }

    if !servers.is_empty() && !servers.iter().any(|server| server.default) {
        servers[0].default = true;
    }
}

fn key_passphrase(config: &RemoteServerInput) -> Option<String> {
    match &config.auth {
        RemoteServerAuth::SshKeyPath { passphrase, .. } => {
            passphrase.clone().filter(|value| !value.is_empty())
        }
        RemoteServerAuth::Password { .. } => None,
    }
}

fn uses_ssh_key(config: &RemoteServerInput) -> bool {
    matches!(&config.auth, RemoteServerAuth::SshKeyPath { .. })
}

fn auth_connection_changed(existing: &RemoteServerAuth, updated: &RemoteServerAuth) -> bool {
    match (existing, updated) {
        (
            RemoteServerAuth::SshKeyPath {
                path: existing_path,
                ..
            },
            RemoteServerAuth::SshKeyPath {
                path: updated_path, ..
            },
        ) => existing_path != updated_path,
        (
            RemoteServerAuth::Password {
                password: existing_password,
            },
            RemoteServerAuth::Password {
                password: updated_password,
            },
        ) => existing_password != updated_password,
        _ => true,
    }
}

fn restore_passphrase(server_id: &str, previous: Option<&str>) -> Result<(), String> {
    match previous {
        Some(previous) => keychain::store_passphrase(server_id, previous),
        None => keychain::delete_passphrase(server_id),
    }
}

async fn load_server(app: &AppHandle, server_id: &str) -> Result<RemoteServerConfig, String> {
    let preferences = crate::load_preferences(app.clone()).await?;
    find_server(&preferences.remote_servers, server_id)
}

fn parse_remote_installation(output: &str) -> Option<(String, String)> {
    let mut version = None;
    let mut token = None;

    for line in output.lines() {
        if let Some(value) = line.strip_prefix("JEAN_VERSION=") {
            version = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("JEAN_TOKEN=") {
            token = Some(value.trim().to_string());
        }
    }

    match (version, token) {
        (Some(version), Some(token)) if !version.is_empty() && !token.is_empty() => {
            Some((version, token))
        }
        _ => None,
    }
}

fn inspect_remote_installation(
    app: &AppHandle,
    server: &RemoteServerConfig,
) -> Result<Option<(String, String)>, String> {
    let output = ssh::exec_checked(
        app,
        server,
        r#"if [ -r /opt/jean-remote/VERSION ] && [ -r /etc/systemd/system/jean-remote.service ]; then
  version=$(cat /opt/jean-remote/VERSION)
  token=$(sed -n 's/.*--token \([^[:space:]]*\).*/\1/p' /etc/systemd/system/jean-remote.service | tail -n 1)
  printf 'JEAN_VERSION=%s\nJEAN_TOKEN=%s\n' "$version" "$token"
fi"#,
    )?;
    Ok(parse_remote_installation(&output))
}

async fn verify_server_ssh(
    app: &AppHandle,
    server: &RemoteServerConfig,
) -> Result<RemoteConnectionTest, String> {
    tunnel::set_runtime_status(&server.id, RemoteServerStatus::Connecting, None);
    let app_for_test = app.clone();
    let server_for_test = server.clone();
    let result =
        tokio::task::spawn_blocking(move || ssh::test_connection(&app_for_test, &server_for_test))
            .await
            .map_err(|e| format!("SSH connection test task failed: {e}"))??;

    if result.success {
        // An SSH test is a one-off command, not a live tunnel. Only mark the
        // server reachable — Connected is reserved for an open tunnel.
        tunnel::set_runtime_status(&server.id, RemoteServerStatus::Reachable, None);
    } else {
        tunnel::set_runtime_status(
            &server.id,
            RemoteServerStatus::Error,
            Some(result.message.clone()),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn add_remote_server(
    app: AppHandle,
    config: RemoteServerInput,
) -> Result<RemoteServerConfig, String> {
    config.validate()?;
    let mut preferences = crate::load_preferences(app.clone()).await?;
    let should_be_default = config.default || preferences.remote_servers.is_empty();
    let server_id = Uuid::new_v4().to_string();
    let passphrase = key_passphrase(&config);
    if let Some(passphrase) = passphrase.as_deref() {
        keychain::store_passphrase(&server_id, passphrase)?;
    }
    let mut server = config.into_config(server_id.clone());
    server.default = should_be_default;

    if should_be_default {
        for existing in &mut preferences.remote_servers {
            existing.default = false;
        }
    }
    preferences.remote_servers.push(server.clone());
    if let Err(error) = crate::save_preferences(app.clone(), preferences).await {
        if passphrase.is_some() {
            let _ = keychain::delete_passphrase(&server_id);
        }
        return Err(error);
    }

    match verify_server_ssh(&app, &server).await {
        Ok(result) if result.success => {
            let app_for_inspection = app.clone();
            let server_for_inspection = server.clone();
            if let Ok(Some((version, token))) = tokio::task::spawn_blocking(move || {
                inspect_remote_installation(&app_for_inspection, &server_for_inspection)
            })
            .await
            .map_err(|e| format!("Remote installation inspection task failed: {e}"))
            .and_then(|result| result)
            {
                let mut preferences = crate::load_preferences(app.clone()).await?;
                if let Some(stored) = preferences
                    .remote_servers
                    .iter_mut()
                    .find(|candidate| candidate.id == server_id)
                {
                    stored.installed_version = Some(version.clone());
                    stored.http_token = Some(token.clone());
                }
                crate::save_preferences(app, preferences).await?;
                server.installed_version = Some(version);
                server.http_token = Some(token);
            }
            server.status = RemoteServerStatus::Reachable;
        }
        Ok(_) => {
            server.status = RemoteServerStatus::Error;
        }
        Err(error) => {
            tunnel::set_runtime_status(&server_id, RemoteServerStatus::Error, Some(error));
            server.status = RemoteServerStatus::Error;
        }
    }
    Ok(server)
}

#[tauri::command]
pub async fn update_remote_server(
    app: AppHandle,
    server_id: String,
    config: RemoteServerInput,
) -> Result<RemoteServerConfig, String> {
    config.validate()?;
    let mut preferences = crate::load_preferences(app.clone()).await?;
    let existing_index = preferences
        .remote_servers
        .iter()
        .position(|server| server.id == server_id)
        .ok_or_else(|| format!("Remote server not found: {server_id}"))?;
    let existing = preferences.remote_servers[existing_index].clone();
    let passphrase = key_passphrase(&config);
    let keychain_changes = passphrase.is_some() || !uses_ssh_key(&config);
    let previous_passphrase = if keychain_changes {
        keychain::load_passphrase(&server_id)?
    } else {
        None
    };
    if let Some(passphrase) = passphrase.as_deref() {
        keychain::store_passphrase(&server_id, passphrase)?;
    } else if !uses_ssh_key(&config) {
        keychain::delete_passphrase(&server_id)?;
    }
    let connection_changed = existing.host != config.host.trim()
        || existing.port != config.port
        || existing.username != config.username.trim()
        || auth_connection_changed(&existing.auth, &config.auth)
        || existing.remote_port != config.remote_port;

    let mut updated = config.into_config(server_id.clone());
    if !connection_changed {
        updated.http_token = existing.http_token;
        updated.installed_version = existing.installed_version;
    } else {
        tunnel::forget(&server_id);
    }
    preferences.remote_servers[existing_index] = updated.clone();
    normalize_default(
        &mut preferences.remote_servers,
        updated.default.then_some(server_id.as_str()),
    );
    updated.default = preferences.remote_servers[existing_index].default;
    if let Err(error) = crate::save_preferences(app, preferences).await {
        if keychain_changes {
            let rollback = restore_passphrase(&server_id, previous_passphrase.as_deref());
            if let Err(rollback_error) = rollback {
                return Err(format!(
                    "{error}. Keychain rollback also failed: {rollback_error}"
                ));
            }
        }
        return Err(error);
    }
    Ok(updated)
}

#[tauri::command]
pub async fn remove_remote_server(app: AppHandle, server_id: String) -> Result<(), String> {
    let mut preferences = crate::load_preferences(app.clone()).await?;
    let previous_len = preferences.remote_servers.len();
    preferences
        .remote_servers
        .retain(|server| server.id != server_id);
    if preferences.remote_servers.len() == previous_len {
        return Err(format!("Remote server not found: {server_id}"));
    }
    let previous_passphrase = keychain::load_passphrase(&server_id)?;
    keychain::delete_passphrase(&server_id)?;
    tunnel::forget(&server_id);
    normalize_default(&mut preferences.remote_servers, None);
    if let Err(error) = crate::save_preferences(app, preferences).await {
        if let Err(rollback_error) = restore_passphrase(&server_id, previous_passphrase.as_deref())
        {
            return Err(format!(
                "{error}. Keychain rollback also failed: {rollback_error}"
            ));
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub async fn list_remote_servers(app: AppHandle) -> Result<Vec<RemoteServerConfig>, String> {
    let mut preferences = crate::load_preferences(app).await?;
    for server in &mut preferences.remote_servers {
        server.status = tunnel::status(server).status;
    }
    Ok(preferences.remote_servers)
}

#[tauri::command]
pub async fn test_remote_server(
    app: AppHandle,
    server_id: String,
) -> Result<RemoteConnectionTest, String> {
    let server = load_server(&app, &server_id).await?;
    verify_server_ssh(&app, &server).await.inspect_err(|error| {
        tunnel::set_runtime_status(&server_id, RemoteServerStatus::Error, Some(error.clone()));
    })
}

#[tauri::command]
pub async fn list_remote_jean_versions() -> Result<Vec<RemoteJeanVersionInfo>, String> {
    provision::list_available_versions().await
}

#[tauri::command]
pub async fn provision_remote_server(
    app: AppHandle,
    server_id: String,
    version: Option<String>,
) -> Result<ProvisionResult, String> {
    let server = load_server(&app, &server_id).await?;
    tunnel::set_runtime_status(&server_id, RemoteServerStatus::Provisioning, None);
    let token = crate::http_server::auth::generate_token();

    let result = match provision::provision(&app, &server, &token, version.as_deref()).await {
        Ok(result) => result,
        Err(error) => {
            tunnel::set_runtime_status(&server_id, RemoteServerStatus::Error, Some(error.clone()));
            return Err(error);
        }
    };

    // Read the token back from the remote service file so we persist the value
    // the service is actually running with, not just the one we generated. Falls
    // back to the generated token if the read-back fails.
    let app_for_readback = app.clone();
    let server_for_readback = server.clone();
    let readback_token = tokio::task::spawn_blocking(move || {
        inspect_remote_installation(&app_for_readback, &server_for_readback)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .flatten()
    .map(|(_, token)| token);
    let token = readback_token.unwrap_or(token);

    let persist_result = async {
        let mut preferences = crate::load_preferences(app.clone()).await?;
        let stored = preferences
            .remote_servers
            .iter_mut()
            .find(|candidate| candidate.id == server_id)
            .ok_or_else(|| format!("Remote server was removed during provisioning: {server_id}"))?;
        stored.http_token = Some(token);
        stored.installed_version = Some(result.version.clone());
        // Provisioning verified SSH end to end; the server is reachable (and now
        // provisioned) even though no tunnel is open yet.
        stored.status = RemoteServerStatus::Reachable;
        crate::save_preferences(app, preferences).await
    }
    .await;
    if let Err(error) = persist_result {
        let error = format!(
            "Jean was provisioned, but its connection token could not be saved: {error}. Provision the server again before connecting."
        );
        tunnel::set_runtime_status(&server_id, RemoteServerStatus::Error, Some(error.clone()));
        return Err(error);
    }
    tunnel::set_runtime_status(&server_id, RemoteServerStatus::Reachable, None);
    Ok(result)
}

/// Re-read the token the remote service is actually launched with and, if it
/// differs from the one stored locally, persist the corrected value. Returns the
/// updated server when a divergence was repaired. This recovers from a stale
/// local token after the server was re-provisioned (or provisioned by an older
/// Jean that left a different token running).
async fn resync_remote_token(
    app: &AppHandle,
    server: &RemoteServerConfig,
) -> Result<Option<RemoteServerConfig>, String> {
    let app_for_inspect = app.clone();
    let server_for_inspect = server.clone();
    let inspected = tokio::task::spawn_blocking(move || {
        inspect_remote_installation(&app_for_inspect, &server_for_inspect)
    })
    .await
    .map_err(|e| format!("Remote token inspection task failed: {e}"))??;

    let Some((version, token)) = inspected else {
        return Ok(None);
    };
    if server.http_token.as_deref() == Some(token.as_str()) {
        // Stored token already matches the remote — nothing to persist.
        return Ok(None);
    }

    let mut preferences = crate::load_preferences(app.clone()).await?;
    let Some(stored) = preferences
        .remote_servers
        .iter_mut()
        .find(|candidate| candidate.id == server.id)
    else {
        return Ok(None);
    };
    stored.http_token = Some(token.clone());
    stored.installed_version = Some(version.clone());
    crate::save_preferences(app.clone(), preferences).await?;

    let mut updated = server.clone();
    updated.http_token = Some(token);
    updated.installed_version = Some(version);
    Ok(Some(updated))
}

#[tauri::command]
pub async fn connect_remote_server(
    app: AppHandle,
    server_id: String,
) -> Result<RemoteConnection, String> {
    let server = load_server(&app, &server_id).await?;
    // The remote service file is the source of truth for the token. Re-sync it
    // before connecting so a re-provisioned/rotated token (or a server that was
    // provisioned outside this install) can't leave us with a stale one. A failed
    // inspection is non-fatal — fall back to the stored token and let the tunnel
    // health check surface any real mismatch.
    let server = match resync_remote_token(&app, &server).await {
        Ok(Some(updated)) => updated,
        Ok(None) => server,
        Err(error) => {
            log::warn!("Remote token re-sync failed before connect: {error}");
            server
        }
    };
    tunnel::connect(&app, &server).await.map_err(|error| {
        if error.contains("401") {
            format!(
                "Remote Jean rejected the connection token (401). The running service \
                 is using a different token than its config file — re-provision the \
                 server to reset it. ({error})"
            )
        } else {
            error
        }
    })
}

#[tauri::command]
pub async fn disconnect_remote_server(server_id: String) -> Result<(), String> {
    tunnel::disconnect(&server_id)
}

/// Called by the frontend when it opened the tunnel but failed to register the
/// remote WebSocket transport. Tears the tunnel down and marks the server
/// errored so status polling stops reporting it as connected.
#[tauri::command]
pub async fn report_remote_connection_failure(
    server_id: String,
    error: String,
) -> Result<(), String> {
    tunnel::mark_connection_failed(&server_id, error);
    Ok(())
}

#[tauri::command]
pub async fn get_remote_server_status(
    app: AppHandle,
    server_id: String,
) -> Result<RemoteServerStatusInfo, String> {
    let server = load_server(&app, &server_id).await?;
    Ok(tunnel::status(&server))
}

#[tauri::command]
pub async fn check_remote_server_health(
    app: AppHandle,
    server_id: String,
) -> Result<RemoteServerStatusInfo, String> {
    let server = load_server(&app, &server_id).await?;
    if server.http_token.is_some() {
        tunnel::check_health(&server).await;
    } else if let Err(error) = verify_server_ssh(&app, &server).await {
        tunnel::set_runtime_status(&server_id, RemoteServerStatus::Error, Some(error));
    }
    Ok(tunnel::status(&server))
}

#[derive(serde::Serialize)]
pub struct LocalToolStatus {
    pub claude_cli: bool,
    pub gh_cli: bool,
}

#[tauri::command]
pub async fn get_local_tool_status(app: AppHandle) -> LocalToolStatus {
    let claude_cli = crate::claude_cli::resolve_cli_binary(&app).exists();
    let gh_cli = crate::platform::find_cli_in_host_path("gh", None).is_some();
    LocalToolStatus { claude_cli, gh_cli }
}

#[tauri::command]
pub async fn install_gh_on_remote(app: AppHandle, server_id: String) -> Result<(), String> {
    let server = load_server(&app, &server_id).await?;
    let script = r#"set -eu
if command -v gh >/dev/null 2>&1; then
  echo "gh already installed: $(gh --version | head -1)"
  exit 0
fi
# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
  x86_64) GH_ARCH="amd64" ;;
  aarch64|arm64) GH_ARCH="arm64" ;;
  *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac
# Try apt/deb first (Debian/Ubuntu)
if command -v apt-get >/dev/null 2>&1; then
  (type -p curl >/dev/null || apt-get install -y curl) 2>/dev/null
  curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
    | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg 2>/dev/null
  chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg
  echo "deb [arch=${GH_ARCH} signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
    | tee /etc/apt/sources.list.d/github-cli.list >/dev/null
  apt-get update -qq && apt-get install -y gh
  echo "GitHub CLI installed: $(gh --version | head -1)"
  exit 0
fi
# Fallback: download binary from GitHub releases
GH_VERSION=$(curl -s https://api.github.com/repos/cli/cli/releases/latest | grep '"tag_name"' | cut -d '"' -f 4 | sed 's/v//')
curl -L -o /tmp/gh.tar.gz "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz"
tar xf /tmp/gh.tar.gz -C /tmp
mv "/tmp/gh_${GH_VERSION}_linux_${GH_ARCH}/bin/gh" /usr/local/bin/gh
chmod +x /usr/local/bin/gh
rm -rf /tmp/gh.tar.gz "/tmp/gh_${GH_VERSION}_linux_${GH_ARCH}"
echo "GitHub CLI installed: $(gh --version | head -1)"
"#;
    let output = tokio::task::spawn_blocking(move || ssh::exec(&app, &server, script))
        .await
        .map_err(|e| format!("SSH task failed: {e}"))??;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("gh install failed with status {}", output.status)
        } else {
            format!("gh install failed: {stderr}")
        })
    }
}

#[tauri::command]
pub async fn clone_project_to_remote(
    app: AppHandle,
    project_id: String,
    server_id: String,
    remote_path: Option<String>,
    copy_env_file: Option<bool>,
) -> Result<crate::projects::types::RemoteClone, String> {
    super::clone::clone_project_to_remote(app, project_id, server_id, remote_path, copy_env_file)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::types::{RemoteServerAuth, RemoteServerStatus};

    fn server(id: &str, default: bool) -> RemoteServerConfig {
        RemoteServerConfig {
            id: id.to_string(),
            name: id.to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "jean".to_string(),
            auth: RemoteServerAuth::SshKeyPath {
                path: "/tmp/key".to_string(),
                passphrase: None,
            },
            default,
            remote_port: 3456,
            status: RemoteServerStatus::Disconnected,
            http_token: None,
            installed_version: None,
        }
    }

    #[test]
    fn default_normalization_keeps_exactly_one_preferred_server() {
        let mut servers = vec![server("one", true), server("two", false)];
        normalize_default(&mut servers, Some("two"));
        assert!(!servers[0].default);
        assert!(servers[1].default);
    }

    #[test]
    fn default_normalization_promotes_first_when_needed() {
        let mut servers = vec![server("one", false), server("two", false)];
        normalize_default(&mut servers, None);
        assert!(servers[0].default);
        assert!(!servers[1].default);
    }

    #[test]
    fn parses_existing_remote_installation() {
        assert_eq!(
            parse_remote_installation("JEAN_VERSION=0.1.60\nJEAN_TOKEN=existing-token\n"),
            Some(("0.1.60".to_string(), "existing-token".to_string()))
        );
        assert_eq!(parse_remote_installation("JEAN_VERSION=0.1.60\n"), None);
    }
}
