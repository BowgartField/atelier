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
    if let Err(error) = crate::save_preferences(app, preferences).await {
        if passphrase.is_some() {
            let _ = keychain::delete_passphrase(&server_id);
        }
        return Err(error);
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
    tunnel::set_runtime_status(&server_id, RemoteServerStatus::Connecting, None);
    let app_for_test = app.clone();
    let server_for_test = server.clone();
    let task_result =
        tokio::task::spawn_blocking(move || ssh::test_connection(&app_for_test, &server_for_test))
            .await
            .map_err(|e| format!("SSH connection test task failed: {e}"))
            .and_then(|result| result);
    let result = match task_result {
        Ok(result) => result,
        Err(error) => {
            tunnel::set_runtime_status(&server_id, RemoteServerStatus::Error, Some(error.clone()));
            return Err(error);
        }
    };

    if result.success {
        tunnel::set_runtime_status(&server_id, RemoteServerStatus::Disconnected, None);
    } else {
        tunnel::set_runtime_status(
            &server_id,
            RemoteServerStatus::Error,
            Some(result.message.clone()),
        );
    }
    Ok(result)
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

    let persist_result = async {
        let mut preferences = crate::load_preferences(app.clone()).await?;
        let stored = preferences
            .remote_servers
            .iter_mut()
            .find(|candidate| candidate.id == server_id)
            .ok_or_else(|| format!("Remote server was removed during provisioning: {server_id}"))?;
        stored.http_token = Some(token);
        stored.installed_version = Some(result.version.clone());
        stored.status = RemoteServerStatus::Disconnected;
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
    tunnel::set_runtime_status(&server_id, RemoteServerStatus::Disconnected, None);
    Ok(result)
}

#[tauri::command]
pub async fn connect_remote_server(
    app: AppHandle,
    server_id: String,
) -> Result<RemoteConnection, String> {
    let server = load_server(&app, &server_id).await?;
    tunnel::connect(&app, &server).await
}

#[tauri::command]
pub async fn disconnect_remote_server(server_id: String) -> Result<(), String> {
    tunnel::disconnect(&server_id)
}

#[tauri::command]
pub async fn get_remote_server_status(
    app: AppHandle,
    server_id: String,
) -> Result<RemoteServerStatusInfo, String> {
    let server = load_server(&app, &server_id).await?;
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
pub async fn install_gh_on_remote(
    app: AppHandle,
    server_id: String,
) -> Result<(), String> {
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
    let output = tokio::task::spawn_blocking(move || {
        ssh::exec(&app, &server, script)
    })
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
) -> Result<crate::projects::types::RemoteClone, String> {
    super::clone::clone_project_to_remote(app, project_id, server_id, remote_path).await
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
}
