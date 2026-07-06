use std::collections::HashMap;
use std::io::Read;
use std::net::TcpListener;
use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;
use serde::Deserialize;
use tauri::AppHandle;

use super::ssh;
use super::types::{
    RemoteConnection, RemoteServerConfig, RemoteServerStatus, RemoteServerStatusInfo,
};

struct TunnelProcess {
    child: Child,
    local_port: u16,
    remote_port: u16,
}

#[derive(Clone)]
struct RuntimeState {
    status: RemoteServerStatus,
    last_error: Option<String>,
}

static TUNNELS: Lazy<Mutex<HashMap<String, TunnelProcess>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static RUNTIME_STATES: Lazy<Mutex<HashMap<String, RuntimeState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    ok: bool,
    app_version: Option<String>,
}

fn reserve_local_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("Failed to reserve a local tunnel port: {e}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|e| format!("Failed to read local tunnel port: {e}"))
}

pub fn set_runtime_status(server_id: &str, status: RemoteServerStatus, last_error: Option<String>) {
    RUNTIME_STATES
        .lock()
        .unwrap()
        .insert(server_id.to_string(), RuntimeState { status, last_error });
}

fn child_exit_error(tunnel: &mut TunnelProcess) -> Option<String> {
    match tunnel.child.try_wait() {
        Ok(Some(status)) => {
            let mut stderr = String::new();
            if let Some(mut pipe) = tunnel.child.stderr.take() {
                let _ = pipe.read_to_string(&mut stderr);
            }
            Some(if stderr.trim().is_empty() {
                format!("SSH tunnel exited with status {status}")
            } else {
                format!("SSH tunnel exited: {}", stderr.trim())
            })
        }
        Ok(None) => None,
        Err(error) => Some(format!("Failed to inspect SSH tunnel: {error}")),
    }
}

fn remove_tunnel(server_id: &str, terminate: bool) -> Result<(), String> {
    let tunnel = TUNNELS.lock().unwrap().remove(server_id);
    if let Some(mut tunnel) = tunnel {
        if terminate {
            match tunnel.child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => tunnel
                    .child
                    .kill()
                    .map_err(|e| format!("Failed to stop SSH tunnel: {e}"))?,
                Err(e) => return Err(format!("Failed to inspect SSH tunnel: {e}")),
            }
        }
        let _ = tunnel.child.wait();
    }
    Ok(())
}

fn active_connection(server: &RemoteServerConfig, token: &str) -> Option<RemoteConnection> {
    let mut tunnels = TUNNELS.lock().unwrap();
    let tunnel = tunnels.get_mut(&server.id)?;
    if let Some(error) = child_exit_error(tunnel) {
        tunnels.remove(&server.id);
        set_runtime_status(&server.id, RemoteServerStatus::Error, Some(error));
        return None;
    }
    Some(RemoteConnection {
        server_id: server.id.clone(),
        local_port: tunnel.local_port,
        remote_port: tunnel.remote_port,
        token: token.to_string(),
        url: format!("http://127.0.0.1:{}", tunnel.local_port),
    })
}

async fn wait_for_health(server_id: &str, local_port: u16, token: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| format!("Failed to create tunnel health client: {e}"))?;
    let url = format!("http://127.0.0.1:{local_port}/api/auth");
    let mut last_error = "remote Jean did not answer".to_string();

    for _ in 0..60 {
        {
            let mut tunnels = TUNNELS.lock().unwrap();
            let Some(tunnel) = tunnels.get_mut(server_id) else {
                return Err("SSH tunnel disappeared while connecting".to_string());
            };
            if let Some(error) = child_exit_error(tunnel) {
                return Err(error);
            }
        }

        match client.get(&url).query(&[("token", token)]).send().await {
            Ok(response) if response.status().is_success() => {
                let auth = response
                    .json::<AuthResponse>()
                    .await
                    .map_err(|e| format!("Invalid remote Jean health response: {e}"))?;
                if !auth.ok {
                    return Err("Remote Jean rejected its provisioned token".to_string());
                }
                if let Some(remote_version) = auth.app_version {
                    let local_version = env!("CARGO_PKG_VERSION");
                    if remote_version != local_version {
                        // Debug builds run ahead of the next published release, so
                        // requiring an exact match would make every dev build
                        // unable to connect to any provisioned server. Release
                        // builds keep the strict check.
                        if cfg!(debug_assertions) {
                            log::warn!(
                                "Jean version mismatch ignored in debug build: local {local_version}, remote {remote_version}"
                            );
                        } else {
                            return Err(format!(
                                "Jean version mismatch: local {local_version}, remote {remote_version}"
                            ));
                        }
                    }
                }
                return Ok(());
            }
            Ok(response) => {
                last_error = format!("remote Jean returned HTTP {}", response.status());
            }
            Err(error) => {
                last_error = error.to_string();
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    Err(format!("Tunnel health check timed out: {last_error}"))
}

pub async fn check_health(server: &RemoteServerConfig) {
    let Some(token) = server
        .http_token
        .as_deref()
        .filter(|token| !token.is_empty())
    else {
        return;
    };
    let Some(connection) = active_connection(server, token) else {
        set_runtime_status(
            &server.id,
            RemoteServerStatus::Error,
            Some("Remote backend tunnel is not connected".to_string()),
        );
        return;
    };

    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| format!("Failed to create remote health client: {e}"))?;
        let response = client
            .get(format!("{}/api/auth", connection.url))
            .query(&[("token", token)])
            .send()
            .await
            .map_err(|e| format!("Remote Jean health check failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("Remote Jean returned HTTP {}", response.status()));
        }
        let auth = response
            .json::<AuthResponse>()
            .await
            .map_err(|e| format!("Invalid remote Jean health response: {e}"))?;
        if !auth.ok {
            return Err("Remote Jean rejected its provisioned token".to_string());
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => set_runtime_status(&server.id, RemoteServerStatus::Connected, None),
        Err(error) => {
            set_runtime_status(&server.id, RemoteServerStatus::Error, Some(error));
        }
    }
}

pub async fn connect(
    app: &AppHandle,
    server: &RemoteServerConfig,
) -> Result<RemoteConnection, String> {
    let token = server
        .http_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "Remote server must be provisioned before connecting".to_string())?;

    if let Some(connection) = active_connection(server, token) {
        set_runtime_status(&server.id, RemoteServerStatus::Connecting, None);
        match wait_for_health(&server.id, connection.local_port, token).await {
            Ok(()) => {
                set_runtime_status(&server.id, RemoteServerStatus::Connected, None);
                return Ok(connection);
            }
            Err(error) => {
                set_runtime_status(&server.id, RemoteServerStatus::Error, Some(error.clone()));
                return Err(error);
            }
        }
    }

    set_runtime_status(&server.id, RemoteServerStatus::Connecting, None);
    let startup = (|| {
        let local_port = reserve_local_port()?;
        let child = ssh::build_tunnel_command(app, server, local_port)?
            .spawn()
            .map_err(|e| format!("Failed to start SSH tunnel: {e}"))?;
        Ok::<_, String>((local_port, child))
    })();
    let (local_port, child) = match startup {
        Ok(startup) => startup,
        Err(error) => {
            set_runtime_status(&server.id, RemoteServerStatus::Error, Some(error.clone()));
            return Err(error);
        }
    };
    TUNNELS.lock().unwrap().insert(
        server.id.clone(),
        TunnelProcess {
            child,
            local_port,
            remote_port: server.remote_port,
        },
    );

    if let Err(error) = wait_for_health(&server.id, local_port, token).await {
        let _ = remove_tunnel(&server.id, true);
        set_runtime_status(&server.id, RemoteServerStatus::Error, Some(error.clone()));
        return Err(error);
    }

    set_runtime_status(&server.id, RemoteServerStatus::Connected, None);
    Ok(RemoteConnection {
        server_id: server.id.clone(),
        local_port,
        remote_port: server.remote_port,
        token: token.to_string(),
        url: format!("http://127.0.0.1:{local_port}"),
    })
}

pub fn disconnect(server_id: &str) -> Result<(), String> {
    remove_tunnel(server_id, true)?;
    set_runtime_status(server_id, RemoteServerStatus::Disconnected, None);
    Ok(())
}

/// Tear down the tunnel and mark the server as errored. Used when the SSH tunnel
/// came up but the desktop webview could not establish its data channel — the
/// tunnel is healthy from Rust's view, so a plain disconnect would report
/// `Disconnected` and status polling would repaint the card as connected.
pub fn mark_connection_failed(server_id: &str, error: String) {
    let _ = remove_tunnel(server_id, true);
    set_runtime_status(server_id, RemoteServerStatus::Error, Some(error));
}

pub fn forget(server_id: &str) {
    let _ = remove_tunnel(server_id, true);
    RUNTIME_STATES.lock().unwrap().remove(server_id);
}

pub fn cleanup_all() -> usize {
    let tunnels: Vec<_> = TUNNELS.lock().unwrap().drain().collect();
    let count = tunnels.len();
    for (_, mut tunnel) in tunnels {
        if tunnel.child.try_wait().ok().flatten().is_none() {
            let _ = tunnel.child.kill();
        }
        let _ = tunnel.child.wait();
    }
    RUNTIME_STATES.lock().unwrap().clear();
    count
}

pub fn status(server: &RemoteServerConfig) -> RemoteServerStatusInfo {
    let mut local_port = None;
    let mut detected_error = None;
    {
        let mut tunnels = TUNNELS.lock().unwrap();
        if let Some(tunnel) = tunnels.get_mut(&server.id) {
            if let Some(error) = child_exit_error(tunnel) {
                detected_error = Some(error);
            } else {
                local_port = Some(tunnel.local_port);
            }
        }
        if detected_error.is_some() {
            tunnels.remove(&server.id);
        }
    }

    if let Some(error) = detected_error {
        set_runtime_status(&server.id, RemoteServerStatus::Error, Some(error.clone()));
    }

    let runtime = RUNTIME_STATES.lock().unwrap().get(&server.id).cloned();
    RemoteServerStatusInfo {
        server_id: server.id.clone(),
        status: runtime
            .as_ref()
            .map(|state| state.status.clone())
            .unwrap_or_else(|| {
                if local_port.is_some() {
                    RemoteServerStatus::Connected
                } else {
                    RemoteServerStatus::Disconnected
                }
            }),
        local_port,
        remote_port: server.remote_port,
        last_error: runtime.and_then(|state| state.last_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_port_is_loopback_and_available() {
        let port = reserve_local_port().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        assert_eq!(listener.local_addr().unwrap().ip().to_string(), "127.0.0.1");
    }
}
