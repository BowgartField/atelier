use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use flate2::read::GzDecoder;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::ssh;
use super::types::{ProvisionResult, RemoteJeanVersionInfo, RemoteServerConfig};
use crate::http_server::EmitExt;

const JEAN_UPDATER_PUBLIC_KEY: &str =
    "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYyNzkyNTI0QUFENzA3MUYKUldRZkI5ZXFKQ1Y1WWdod05PSjhkcUVBUnNyOWJTcEpVazBRN01SUndya2JQcTdNeUxrS0pFY3QK";
const JEAN_REPO: &str = "coollabsio/jean";
const SERVICE_NAME: &str = "jean-remote.service";
const REMOTE_INSTALL_DIR: &str = "/opt/jean-remote";
const REMOTE_BINARY_PATH: &str = "/opt/jean-remote/jean.AppImage";
const PROVISION_PROGRESS_EVENT: &str = "remote-server:provision-progress";
const PROVISION_LOG_EVENT: &str = "remote-server:provision-log";

#[derive(Debug, Clone, Serialize)]
struct ProvisionProgressEvent {
    server_id: String,
    stage: String,
    message: String,
    percent: u8,
}

#[derive(Debug, Clone, Serialize)]
struct ProvisionLogEvent {
    server_id: String,
    stream: String,
    line: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    version: String,
    platforms: HashMap<String, ReleasePlatform>,
}

#[derive(Debug, Deserialize)]
struct ReleasePlatform {
    url: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    published_at: String,
    draft: bool,
    prerelease: bool,
}

pub async fn list_available_versions() -> Result<Vec<RemoteJeanVersionInfo>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(format!("Jean/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Failed to create release list client: {e}"))?;
    let releases = client
        .get(format!(
            "https://api.github.com/repos/{JEAN_REPO}/releases?per_page=30"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to list Jean releases: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Jean release list request failed: {e}"))?
        .json::<Vec<GithubRelease>>()
        .await
        .map_err(|e| format!("Failed to parse Jean release list: {e}"))?;

    Ok(releases
        .into_iter()
        .filter(|release| !release.draft)
        .map(|release| RemoteJeanVersionInfo {
            version: release.tag_name.trim_start_matches('v').to_string(),
            published_at: release.published_at,
            prerelease: release.prerelease,
        })
        .collect())
}

pub fn jean_launch_command(remote_port: u16, token: &str) -> String {
    format!(
        "/usr/bin/xvfb-run -a {REMOTE_BINARY_PATH} --headless --host 127.0.0.1 --port {remote_port} --token {token}"
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn emit_progress(app: &AppHandle, server_id: &str, stage: &str, message: &str, percent: u8) {
    let _ = app.emit_all(
        PROVISION_PROGRESS_EVENT,
        &ProvisionProgressEvent {
            server_id: server_id.to_string(),
            stage: stage.to_string(),
            message: message.to_string(),
            percent,
        },
    );
}

fn emit_log(app: &AppHandle, server_id: &str, stream: &str, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let _ = app.emit_all(
        PROVISION_LOG_EVENT,
        &ProvisionLogEvent {
            server_id: server_id.to_string(),
            stream: stream.to_string(),
            line: line.to_string(),
        },
    );
}

fn emit_output(app: &AppHandle, server_id: &str, output: &std::process::Output) {
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        emit_log(app, server_id, "stdout", line);
    }
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        emit_log(app, server_id, "stderr", line);
    }
}

fn dependency_install_command() -> &'static str {
    r#"set -eu
if [ "$(uname -s)" != "Linux" ]; then
  echo "Remote provisioning currently supports Linux servers only" >&2
  exit 64
fi
if [ "$(id -u)" -eq 0 ]; then
  SUDO=""
elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then
  SUDO="sudo -n"
else
  echo "Passwordless sudo or root access is required for provisioning" >&2
  exit 77
fi
if command -v apt-get >/dev/null 2>&1; then
  $SUDO apt-get update -qq
  WEBKIT_PACKAGE="libwebkit2gtk-4.1-0"
  if ! apt-cache show "$WEBKIT_PACKAGE" >/dev/null 2>&1; then
    WEBKIT_PACKAGE="libwebkit2gtk-4.0-37"
  fi
  $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y curl xvfb libgtk-3-0 "$WEBKIT_PACKAGE"
elif command -v dnf >/dev/null 2>&1; then
  $SUDO dnf install -y curl xorg-x11-server-Xvfb gtk3 webkit2gtk4.1
elif command -v yum >/dev/null 2>&1; then
  $SUDO yum install -y curl xorg-x11-server-Xvfb gtk3 webkit2gtk4.1
elif command -v pacman >/dev/null 2>&1; then
  $SUDO pacman -Sy --noconfirm curl xorg-server-xvfb gtk3 webkit2gtk-4.1
else
  echo "Unsupported Linux package manager" >&2
  exit 65
fi
command -v systemctl >/dev/null 2>&1 || {
  echo "systemd is required for Jean remote provisioning" >&2
  exit 66
}
"#
}

fn platform_key(architecture: &str) -> Result<&'static str, String> {
    match architecture.trim() {
        "x86_64" | "amd64" => Ok("linux-x86_64"),
        "aarch64" | "arm64" => Ok("linux-aarch64"),
        other => Err(format!("Unsupported remote architecture: {other}")),
    }
}

fn decode_base64_text(value: &str, label: &str) -> Result<String, String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|e| format!("Invalid {label} encoding: {e}"))?;
    String::from_utf8(decoded).map_err(|e| format!("Invalid {label} text: {e}"))
}

fn verify_artifact(bytes: &[u8], release_signature: &str) -> Result<(), String> {
    let public_key_text = decode_base64_text(JEAN_UPDATER_PUBLIC_KEY, "updater public key")?;
    let public_key = PublicKey::decode(&public_key_text)
        .map_err(|e| format!("Invalid updater public key: {e}"))?;
    let signature_text = decode_base64_text(release_signature, "release signature")?;
    let signature = Signature::decode(&signature_text)
        .map_err(|e| format!("Invalid release signature: {e}"))?;
    public_key
        .verify(bytes, &signature, true)
        .map_err(|e| format!("Jean artifact signature verification failed: {e}"))
}

fn extract_appimage(archive_bytes: &[u8], destination: &Path) -> Result<(), String> {
    let decoder = GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("Failed to read Jean archive: {e}"))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Failed to read Jean archive entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("Invalid Jean archive path: {e}"))?;
        if !path.to_string_lossy().ends_with(".AppImage") {
            continue;
        }
        let mut output = std::fs::File::create(destination)
            .map_err(|e| format!("Failed to create temporary AppImage: {e}"))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|e| format!("Failed to extract Jean AppImage: {e}"))?;
        return Ok(());
    }

    Err("Jean updater archive did not contain an AppImage".to_string())
}

fn artifact_dir(app: &AppHandle, server_id: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {e}"))?
        .join("remote-artifacts")
        .join(server_id)
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create artifact directory: {e}"))?;
    Ok(dir)
}

async fn download_release(
    app: &AppHandle,
    server_id: &str,
    architecture: &str,
    requested_version: &str,
) -> Result<(String, Vec<u8>), String> {
    let manifest_url = format!(
        "https://github.com/{JEAN_REPO}/releases/download/v{requested_version}/latest.json"
    );
    // No overall timeout: large artifacts on slow links would otherwise abort
    // mid-download. A read timeout guards against a genuinely stalled socket.
    let client = reqwest::Client::builder()
        .read_timeout(Duration::from_secs(60))
        .user_agent(format!("Jean/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("Failed to create download client: {e}"))?;
    let manifest = client
        .get(manifest_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download Jean release manifest: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Jean release manifest request failed: {e}"))?
        .json::<ReleaseManifest>()
        .await
        .map_err(|e| format!("Failed to parse Jean release manifest: {e}"))?;
    if manifest.version.trim_start_matches('v') != requested_version {
        return Err(format!(
            "Jean release manifest version mismatch: requested {requested_version}, got {}",
            manifest.version
        ));
    }

    let key = platform_key(architecture)?;
    let platform = manifest
        .platforms
        .get(key)
        .ok_or_else(|| format!("Jean release has no artifact for {key}"))?;
    let mut response = client
        .get(&platform.url)
        .send()
        .await
        .map_err(|e| format!("Failed to download Jean artifact: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Jean artifact request failed: {e}"))?;

    // Stream the artifact so the UI shows real progress instead of freezing on
    // "Downloading Jean release" for the whole (100+ MB) transfer. Map bytes to
    // the 35..=58% slice of the overall provisioning bar.
    let total = response.content_length().filter(|len| *len > 0);
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut last_percent = 35u8;
    let mut last_logged_mb = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed to read Jean artifact: {e}"))?
    {
        bytes.extend_from_slice(&chunk);
        let downloaded = bytes.len() as u64;
        let downloaded_mb = downloaded / 1_048_576;
        if let Some(total) = total {
            let percent = (35 + downloaded.saturating_mul(23) / total).min(58) as u8;
            if percent != last_percent {
                last_percent = percent;
                emit_progress(
                    app,
                    server_id,
                    "downloading_release",
                    &format!("Downloading Jean release ({downloaded_mb}/{} MB)", total / 1_048_576),
                    percent,
                );
            }
        }
        if downloaded_mb >= last_logged_mb + 10 {
            last_logged_mb = downloaded_mb;
            emit_log(
                app,
                server_id,
                "system",
                &format!("Downloaded {downloaded_mb} MB"),
            );
        }
    }
    verify_artifact(&bytes, &platform.signature)?;

    Ok((manifest.version, bytes))
}

fn build_systemd_unit(server: &RemoteServerConfig, token: &str) -> String {
    format!(
        r#"[Unit]
Description=Jean remote headless server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={username}
Environment=APPIMAGE_EXTRACT_AND_RUN=1
# The desktop webview (origin tauri://localhost / https://tauri.localhost)
# reaches this backend cross-origin through the loopback SSH tunnel. Allow any
# origin so its fetch/WebSocket handshake is not blocked by CORS — the server
# still binds 127.0.0.1 only and requires the token on every request.
Environment=JEAN_ALLOWED_ORIGINS=*
ExecStart={launch_command}
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
"#,
        username = server.username,
        launch_command = jean_launch_command(server.remote_port, token),
    )
}

fn service_health_check_command(remote_port: u16, token: &str) -> String {
    let url = shell_quote(&format!(
        "http://127.0.0.1:{remote_port}/api/auth?token={token}"
    ));
    format!(
        r#"set -eu
for attempt in $(seq 1 40); do
  if curl --fail --silent --max-time 2 {url} >/dev/null; then
    echo active
    exit 0
  fi
  sleep 0.25
done
systemctl status {SERVICE_NAME} --no-pager >&2 || true
journalctl -u {SERVICE_NAME} -n 20 --no-pager >&2 || true
echo "Jean remote API did not become ready" >&2
exit 1"#
    )
}

fn install_artifact_and_service(
    app: &AppHandle,
    server: &RemoteServerConfig,
    local_appimage: &Path,
    version: &str,
    token: &str,
) -> Result<(), String> {
    let remote_temp = format!("/tmp/jean-remote-{}.AppImage", uuid::Uuid::new_v4());
    emit_log(
        app,
        &server.id,
        "system",
        &format!("Uploading Jean {} to {}", version, server.host),
    );
    ssh::scp_to(app, server, local_appimage, &remote_temp)?;

    let unit = build_systemd_unit(server, token);
    let unit_base64 = base64::engine::general_purpose::STANDARD.encode(unit.as_bytes());
    let install_command = format!(
        r#"set -eu
if [ "$(id -u)" -eq 0 ]; then SUDO=""; else SUDO="sudo -n"; fi
$SUDO install -d -m 0755 {install_dir}
$SUDO install -m 0755 {remote_temp} {binary_path}
printf '%s' {version} | $SUDO tee {install_dir}/VERSION >/dev/null
printf '%s' {unit_base64} | base64 -d | $SUDO tee /etc/systemd/system/{service_name} >/dev/null
$SUDO systemctl daemon-reload
$SUDO systemctl enable {service_name}
$SUDO systemctl restart {service_name}
rm -f {remote_temp}
"#,
        install_dir = REMOTE_INSTALL_DIR,
        remote_temp = shell_quote(&remote_temp),
        binary_path = REMOTE_BINARY_PATH,
        version = shell_quote(version),
        unit_base64 = shell_quote(&unit_base64),
        service_name = SERVICE_NAME,
    );
    emit_log(
        app,
        &server.id,
        "system",
        "Installing systemd service and starting Jean remote backend",
    );
    let output = ssh::exec(app, server, &install_command)?;
    emit_output(app, &server.id, &output);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!(
                "Failed to install Jean service with status {}",
                output.status
            )
        } else {
            format!("Failed to install Jean service: {stderr}")
        });
    }
    Ok(())
}

pub async fn provision(
    app: &AppHandle,
    server: &RemoteServerConfig,
    token: &str,
    version: Option<&str>,
) -> Result<ProvisionResult, String> {
    let requested_version = version.unwrap_or(env!("CARGO_PKG_VERSION"));
    emit_progress(app, &server.id, "preparing", "Preparing remote server", 3);
    emit_log(
        app,
        &server.id,
        "system",
        "Checking dependency packages and remote privileges",
    );
    let app_for_dependencies = app.clone();
    let server_for_dependencies = server.clone();
    let dependency_output = tokio::task::spawn_blocking(move || {
        ssh::exec(
            &app_for_dependencies,
            &server_for_dependencies,
            dependency_install_command(),
        )
    })
    .await
    .map_err(|e| format!("Provisioning dependency task failed: {e}"))??;
    emit_output(app, &server.id, &dependency_output);
    if !dependency_output.status.success() {
        let stderr = String::from_utf8_lossy(&dependency_output.stderr)
            .trim()
            .to_string();
        return Err(if stderr.is_empty() {
            format!(
                "Dependency installation failed with status {}",
                dependency_output.status
            )
        } else {
            format!("Dependency installation failed: {stderr}")
        });
    }

    emit_progress(
        app,
        &server.id,
        "detecting_architecture",
        "Detecting remote architecture",
        18,
    );
    let app_for_arch = app.clone();
    let server_for_arch = server.clone();
    let architecture_output =
        tokio::task::spawn_blocking(move || ssh::exec(&app_for_arch, &server_for_arch, "uname -m"))
            .await
            .map_err(|e| format!("Architecture detection task failed: {e}"))??;
    emit_output(app, &server.id, &architecture_output);
    if !architecture_output.status.success() {
        let stderr = String::from_utf8_lossy(&architecture_output.stderr)
            .trim()
            .to_string();
        return Err(if stderr.is_empty() {
            format!(
                "Architecture detection failed with status {}",
                architecture_output.status
            )
        } else {
            format!("Architecture detection failed: {stderr}")
        });
    }
    let architecture = String::from_utf8_lossy(&architecture_output.stdout)
        .trim()
        .to_string();

    emit_progress(
        app,
        &server.id,
        "downloading_release",
        "Downloading Jean release",
        35,
    );
    emit_log(
        app,
        &server.id,
        "system",
        &format!("Selecting release artifact for {architecture}"),
    );
    let (version, archive_bytes) =
        download_release(app, &server.id, &architecture, requested_version).await?;
    emit_log(
        app,
        &server.id,
        "system",
        &format!("Downloaded and verified Jean {}", version),
    );
    let temp_dir = artifact_dir(app, &server.id)?;
    let local_appimage = temp_dir.join("jean.AppImage");
    let app_for_install = app.clone();
    let server_for_install = server.clone();
    let version_for_install = version;
    let token_for_install = token.to_string();
    emit_progress(
        app,
        &server.id,
        "uploading_artifact",
        "Uploading Jean artifact",
        60,
    );
    tokio::task::spawn_blocking(move || {
        let operation = (|| {
            extract_appimage(&archive_bytes, &local_appimage)?;
            install_artifact_and_service(
                &app_for_install,
                &server_for_install,
                &local_appimage,
                &version_for_install,
                &token_for_install,
            )?;
            emit_progress(
                &app_for_install,
                &server_for_install.id,
                "verifying_service",
                "Verifying remote service",
                90,
            );
            let health_output = ssh::exec(
                &app_for_install,
                &server_for_install,
                &service_health_check_command(server_for_install.remote_port, &token_for_install),
            )?;
            emit_output(&app_for_install, &server_for_install.id, &health_output);
            if !health_output.status.success() {
                let stderr = String::from_utf8_lossy(&health_output.stderr)
                    .trim()
                    .to_string();
                return Err(if stderr.is_empty() {
                    format!(
                        "Service verification failed with status {}",
                        health_output.status
                    )
                } else {
                    format!("Service verification failed: {stderr}")
                });
            }
            let active = String::from_utf8_lossy(&health_output.stdout)
                .trim()
                .to_string();
            if active != "active" {
                return Err(format!(
                    "Jean remote service did not become active (status: {active})"
                ));
            }
            emit_progress(
                &app_for_install,
                &server_for_install.id,
                "complete",
                "Jean remote backend is running",
                100,
            );
            emit_log(
                &app_for_install,
                &server_for_install.id,
                "system",
                "Jean remote backend started successfully",
            );
            Ok(ProvisionResult {
                success: true,
                version: version_for_install,
                remote_port: server_for_install.remote_port,
                service_name: SERVICE_NAME.to_string(),
            })
        })();
        let _ = std::fs::remove_dir_all(&temp_dir);
        operation
    })
    .await
    .map_err(|e| format!("Jean installation task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::types::{RemoteServerAuth, RemoteServerStatus};

    #[test]
    fn launch_command_wraps_appimage_with_virtual_display() {
        assert_eq!(
            jean_launch_command(5599, "test-token"),
            "/usr/bin/xvfb-run -a /opt/jean-remote/jean.AppImage --headless --host 127.0.0.1 --port 5599 --token test-token"
        );
    }

    #[test]
    fn service_health_check_waits_for_authenticated_api() {
        let command = service_health_check_command(5599, "test-token");
        assert!(command.contains(
            "curl --fail --silent --max-time 2 'http://127.0.0.1:5599/api/auth?token=test-token'"
        ));
        assert!(command.contains("journalctl -u jean-remote.service"));
    }

    #[test]
    fn platform_mapping_accepts_release_architectures() {
        assert_eq!(platform_key("x86_64").unwrap(), "linux-x86_64");
        assert_eq!(platform_key("aarch64").unwrap(), "linux-aarch64");
        assert!(platform_key("riscv64").is_err());
    }

    #[test]
    fn systemd_unit_binds_jean_to_loopback() {
        let server = RemoteServerConfig {
            id: "server-id".to_string(),
            name: "Cloud".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "jean".to_string(),
            auth: RemoteServerAuth::SshKeyPath {
                path: "/tmp/key".to_string(),
                passphrase: None,
            },
            default: false,
            remote_port: 3456,
            status: RemoteServerStatus::Disconnected,
            http_token: None,
            installed_version: None,
        };
        let unit = build_systemd_unit(&server, "secret");
        assert!(unit.contains("User=jean"));
        assert!(unit.contains("--host 127.0.0.1 --port 3456 --token secret"));
        assert!(unit.contains("APPIMAGE_EXTRACT_AND_RUN=1"));
        // Cross-origin CORS must be allowed for the desktop webview tunnel.
        assert!(unit.contains("Environment=JEAN_ALLOWED_ORIGINS=*"));
    }
}
