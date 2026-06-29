use tauri::AppHandle;

use crate::platform::silent_command;
use crate::projects::storage::{load_projects_data, save_projects_data};
use crate::projects::types::RemoteClone;

use super::ssh;

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Resolve SSH config host aliases in a git remote URL.
///
/// Local ~/.ssh/config may define aliases like `github.com-myaccount` that
/// map to `github.com`. The remote server doesn't have this config, so we
/// resolve the alias with `ssh -G` before passing the URL over SSH.
fn resolve_ssh_url_aliases(url: &str) -> String {
    // Only handle SSH-style git URLs: git@<host>:<path>
    let rest = match url.strip_prefix("git@") {
        Some(rest) => rest,
        None => return url.to_string(),
    };
    let Some(colon_pos) = rest.find(':') else {
        return url.to_string();
    };
    let host = &rest[..colon_pos];
    let path = &rest[colon_pos + 1..];

    // Ask ssh for the effective config (resolves Host → Hostname)
    let output = match std::process::Command::new("ssh").args(["-G", host]).output() {
        Ok(o) if o.status.success() => o,
        _ => return url.to_string(),
    };

    let config = String::from_utf8_lossy(&output.stdout);
    let mut real_hostname = host.to_string();
    let mut real_user = String::from("git");

    for line in config.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(val) = lower.strip_prefix("hostname ") {
            real_hostname = val.trim().to_string();
        } else if let Some(val) = lower.strip_prefix("user ") {
            real_user = val.trim().to_string();
        }
    }

    if real_hostname != host {
        format!("{real_user}@{real_hostname}:{path}")
    } else {
        url.to_string()
    }
}

fn get_git_remote_url(project_path: &str) -> Result<String, String> {
    let output = silent_command("git")
        .args(["-C", project_path, "remote", "get-url", "origin"])
        .output()
        .map_err(|e| format!("Failed to run git remote get-url: {e}"))?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            return Ok(url);
        }
    }

    // Fallback: ls-remote --get-url
    let output = silent_command("git")
        .args(["-C", project_path, "ls-remote", "--get-url", "origin"])
        .output()
        .map_err(|e| format!("Failed to run git ls-remote: {e}"))?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // git ls-remote --get-url returns the argument itself when no remote is configured
        if !url.is_empty() && url != "origin" {
            return Ok(url);
        }
    }

    Err("Project has no git remote 'origin' configured".to_string())
}

pub async fn clone_project_to_remote(
    app: AppHandle,
    project_id: String,
    server_id: String,
    remote_path: Option<String>,
) -> Result<RemoteClone, String> {
    // 1. Find project by id
    let data = load_projects_data(&app)?;
    let project = data
        .find_project(&project_id)
        .cloned()
        .ok_or_else(|| format!("Project not found: {project_id}"))?;

    // 2. Find remote server by id
    let preferences = crate::load_preferences(app.clone()).await?;
    let server = preferences
        .remote_servers
        .iter()
        .find(|s| s.id == server_id)
        .cloned()
        .ok_or_else(|| format!("Remote server not found: {server_id}"))?;

    // 3. Idempotency check: if already cloned to this server, return existing entry
    if let Some(existing) = project
        .remote_clones
        .iter()
        .find(|c| c.server_id == server_id)
    {
        return Ok(existing.clone());
    }

    // 4. Get project git remote URL and resolve any local SSH config aliases
    let project_path = project.path.clone();
    let remote_url = tokio::task::spawn_blocking(move || {
        let url = get_git_remote_url(&project_path)?;
        Ok::<_, String>(resolve_ssh_url_aliases(&url))
    })
    .await
    .map_err(|e| format!("Git remote URL task failed: {e}"))??;

    // 5. Determine remote_path
    let resolved_remote_path = remote_path.unwrap_or_else(|| format!("~/jean/{}", project.name));

    // 6. Run SSH exec to clone or fetch.
    // GIT_SSH_COMMAND accepts new host keys so github.com (and others) don't
    // need to be pre-populated in the remote's known_hosts.
    let clone_command = format!(
        "set -eu\nexport GIT_SSH_COMMAND='ssh -o StrictHostKeyChecking=accept-new'\nif [ -d {path}/.git ]; then\n  git -C {path} fetch --all --prune\nelse\n  mkdir -p \"$(dirname {path})\"\n  git clone {url} {path}\nfi",
        path = shell_quote(&resolved_remote_path),
        url = shell_quote(&remote_url),
    );

    let app_for_ssh = app.clone();
    let output = tokio::task::spawn_blocking(move || {
        ssh::exec(&app_for_ssh, &server, &clone_command)
    })
    .await
    .map_err(|e| format!("SSH clone task failed: {e}"))??;

    // 7. Check SSH result
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Remote git clone failed with status {}", output.status)
        } else {
            format!("Remote git clone failed: {stderr}")
        });
    }

    // 8. Save RemoteClone to project
    let clone = RemoteClone {
        server_id: server_id.clone(),
        remote_path: resolved_remote_path,
    };

    let mut data = load_projects_data(&app)?;
    let project_entry = data
        .projects
        .iter_mut()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found when saving clone: {project_id}"))?;
    project_entry.remote_clones.push(clone.clone());
    save_projects_data(&app, &data)?;

    // 9. Return the clone
    Ok(clone)
}
