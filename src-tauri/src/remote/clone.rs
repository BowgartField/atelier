use tauri::AppHandle;

use crate::platform::silent_command;
use crate::projects::storage::{load_projects_data, save_projects_data};
use crate::projects::types::RemoteClone;

use super::ssh;

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Like shell_quote but handles leading `~/` — single quotes prevent tilde
/// expansion so we substitute `$HOME/` and double-quote instead.
fn shell_path(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        let escaped = rest.replace('"', "\\\"");
        format!("\"$HOME/{escaped}\"")
    } else {
        shell_quote(value)
    }
}

struct ResolvedSshUrl {
    url: String,
    /// Identity file from ~/.ssh/config for the original host alias (local path).
    identity_file: Option<String>,
}

/// Resolve SSH config host aliases in a git remote URL.
///
/// Local ~/.ssh/config may define aliases like `github.com-myaccount` that
/// map to `github.com`. The remote server doesn't have this config, so we
/// resolve the alias with `ssh -G` before passing the URL over SSH.
/// Also returns the IdentityFile so callers can ensure it's in the local
/// SSH agent (needed for agent forwarding to work on the remote).
fn resolve_ssh_url_aliases(url: &str) -> ResolvedSshUrl {
    // Only handle SSH-style git URLs: git@<host>:<path>
    let rest = match url.strip_prefix("git@") {
        Some(rest) => rest,
        None => return ResolvedSshUrl { url: url.to_string(), identity_file: None },
    };
    let Some(colon_pos) = rest.find(':') else {
        return ResolvedSshUrl { url: url.to_string(), identity_file: None };
    };
    let host = &rest[..colon_pos];
    let path = &rest[colon_pos + 1..];

    // Ask ssh for the effective config (resolves Host → Hostname)
    let output = match std::process::Command::new("ssh").args(["-G", host]).output() {
        Ok(o) if o.status.success() => o,
        _ => return ResolvedSshUrl { url: url.to_string(), identity_file: None },
    };

    let config = String::from_utf8_lossy(&output.stdout);
    let mut real_hostname = host.to_string();
    let mut real_user = String::from("git");
    let mut identity_file: Option<String> = None;

    for line in config.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(val) = lower.strip_prefix("hostname ") {
            real_hostname = val.trim().to_string();
        } else if let Some(val) = lower.strip_prefix("user ") {
            real_user = val.trim().to_string();
        } else if let Some(val) = lower.strip_prefix("identityfile ") {
            // Take only the first identityfile listed
            if identity_file.is_none() {
                let raw = val.trim();
                // Expand ~ ourselves since this runs in the Tauri process
                let expanded = if let Some(rest) = raw.strip_prefix("~/") {
                    dirs::home_dir()
                        .map(|h| h.join(rest).to_string_lossy().to_string())
                        .unwrap_or_else(|| raw.to_string())
                } else {
                    raw.to_string()
                };
                identity_file = Some(expanded);
            }
        }
    }

    let resolved_url = if real_hostname != host {
        format!("{real_user}@{real_hostname}:{path}")
    } else {
        url.to_string()
    };

    ResolvedSshUrl { url: resolved_url, identity_file }
}

/// Ensure an SSH identity file is loaded into the local ssh-agent so it can
/// be forwarded to the remote server during git clone.
/// Silent on failure — agent forwarding may still work if the key is already
/// loaded via another mechanism (e.g. macOS Keychain agent).
fn ensure_key_in_agent(identity_file: &str) {
    // Check if key is already in agent
    let listed = std::process::Command::new("ssh-add")
        .arg("-l")
        .output()
        .ok();
    if let Some(out) = listed {
        let stdout = String::from_utf8_lossy(&out.stdout);
        // ssh-add -l prints fingerprints; check by filename heuristic
        if stdout.contains(identity_file) {
            return;
        }
    }
    // Attempt to add; --apple-use-keychain loads passphrase from macOS Keychain
    // when available (no-op flag on non-macOS ssh-add versions).
    let _ = std::process::Command::new("ssh-add")
        .args(["--apple-use-keychain", identity_file])
        .output()
        .or_else(|_| {
            std::process::Command::new("ssh-add")
                .arg(identity_file)
                .output()
        });
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

    // 3. Idempotency check: if already cloned to this server with a resolved
    //    absolute path, return the existing entry. Clones saved before the
    //    remote_path resolution fix may still hold an unexpanded `~/...`
    //    path that the remote's own project registry can't read — repair
    //    those by falling through and re-running the clone below (which is
    //    itself idempotent: it fetches instead of re-cloning).
    if let Some(existing) = project
        .remote_clones
        .iter()
        .find(|c| c.server_id == server_id && !c.remote_path.starts_with('~'))
    {
        return Ok(existing.clone());
    }

    // 4. Get project git remote URL, resolve local SSH config aliases, and
    //    ensure the identity key is in the local agent for forwarding.
    let project_path = project.path.clone();
    let remote_url = tokio::task::spawn_blocking(move || {
        let url = get_git_remote_url(&project_path)?;
        let resolved = resolve_ssh_url_aliases(&url);
        if let Some(ref key) = resolved.identity_file {
            ensure_key_in_agent(key);
        }
        Ok::<_, String>(resolved.url)
    })
    .await
    .map_err(|e| format!("Git remote URL task failed: {e}"))??;

    // 5. Determine remote_path
    let resolved_remote_path = remote_path.unwrap_or_else(|| format!("~/jean/{}", project.name));

    // 6. Run SSH exec to clone or fetch. Print the shell-resolved absolute
    //    path afterward — `resolved_remote_path` may contain `~/`, which the
    //    remote Jean process (a plain Rust binary, not a shell) cannot expand
    //    when this path is later passed to `add_project`.
    // GIT_SSH_COMMAND accepts new host keys so github.com (and others) don't
    // need to be pre-populated in the remote's known_hosts.
    let clone_command = format!(
        "set -eu\nexport GIT_SSH_COMMAND='ssh -o StrictHostKeyChecking=accept-new'\nif [ -d {path}/.git ]; then\n  git -C {path} fetch --all --prune\nelse\n  mkdir -p \"$(dirname {path})\"\n  git clone {url} {path}\nfi\necho \"JEAN_REMOTE_PATH:$(cd {path} && pwd)\"",
        path = shell_path(&resolved_remote_path),
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let absolute_remote_path = stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("JEAN_REMOTE_PATH:"))
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or(resolved_remote_path);

    // 8. Save RemoteClone to project
    let clone = RemoteClone {
        server_id: server_id.clone(),
        remote_path: absolute_remote_path,
    };

    let mut data = load_projects_data(&app)?;
    let project_entry = data
        .projects
        .iter_mut()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found when saving clone: {project_id}"))?;
    project_entry
        .remote_clones
        .retain(|c| c.server_id != server_id);
    project_entry.remote_clones.push(clone.clone());
    save_projects_data(&app, &data)?;

    // 9. Return the clone
    Ok(clone)
}
