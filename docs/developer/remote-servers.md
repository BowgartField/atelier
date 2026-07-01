# Remote Servers

Remote servers let the desktop app provision and connect to a headless Jean
backend through an SSH local-forward. Local projects continue to use Tauri IPC,
while project-scoped remote operations use a WebSocket transport associated
with the server that owns the project or worktree.

## Phase 1 backend

The backend lives in `src-tauri/src/remote/`:

- `types.rs` defines persisted snake_case contracts.
- `keychain.rs` stores encrypted-key passphrases in macOS Keychain.
- `ssh.rs` wraps the system `ssh` and `scp` binaries with argument arrays.
- `provision.rs` installs Jean and a systemd service on a Linux server.
- `tunnel.rs` owns live SSH tunnel children and runtime status.
- `commands.rs` exposes persistence, provisioning, connection, and status
  commands.

Every remote command is registered in both `lib.rs` and
`http_server/dispatch.rs`.

## SSH behavior

Command connections use OpenSSH ControlMaster on Unix, with one short socket
path per server under a private, process-specific runtime directory in `/tmp`.
This avoids both OpenSSH config parsing issues with spaces in macOS
`Application Support` paths and Unix socket length limits. Tunnel processes are
independent children so Jean can track and terminate each forward
deterministically. All tracked tunnels are terminated during Jean's exit and
window-close cleanup.

Key authentication passes the configured key as a distinct `-i` argument.
Encrypted key passphrases are stored as generic passwords in macOS Keychain,
keyed by the remote server UUID, and are explicitly omitted from serialized
preferences and command responses. OpenSSH receives the passphrase through an
app-owned `SSH_ASKPASS` helper and a child-only environment variable. Removing a
server removes its Keychain entry. Password authentication uses the same
askpass boundary, but server passwords remain persisted in `preferences.json`;
key authentication is recommended.

SSH targets and user-controlled fields are validated before use. Remote shell
commands are internal templates and dynamic values are shell-quoted or encoded.

## Provisioning

Provisioning currently requires:

- a Linux remote host using apt, dnf, yum, or pacman;
- root access or passwordless sudo;
- systemd;
- x86_64 or aarch64.

The flow installs Xvfb and WebKitGTK/GTK runtime packages, downloads the Linux
artifact from the release manifest for the selected Jean version, verifies its
updater minisign signature with the same public key as the desktop updater,
uploads the extracted AppImage with `scp`, and installs `jean-remote.service`.
The Preferences UI uses a dedicated provisioning modal with a version picker,
step status at the top, and a live log pane at the bottom.

`provision_remote_server` takes an optional `version`; when omitted it falls
back to the desktop's own `CARGO_PKG_VERSION`. `list_remote_jean_versions`
fetches published releases from the GitHub API so the modal can offer a
dropdown. Picking a version other than the desktop's own is only useful for
staging a server ahead of time — `connect_remote_server` still requires the
remote version to exactly match the desktop version (see below), so the
tunnel will refuse to connect until the desktop is upgraded or downgraded to
match.

The backend emits `remote-server:provision-progress` and
`remote-server:provision-log` events during provisioning so the modal can render
the current step and command output without waiting for the final mutation
result.

The current Tauri runtime still initializes GTK before the headless window
configuration is applied, so the AppImage runs behind an Xvfb compatibility
boundary:

```text
xvfb-run -a jean.AppImage --headless --host 127.0.0.1 --port P --token T
```

Provisioning waits for the authenticated `/api/auth` endpoint before reporting
success. A transient `systemctl is-active` result is not sufficient because a
crashing service may already be queued for automatic restart.

The service binds only to remote loopback. It is reachable from the desktop only
through:

```text
ssh -N -L 127.0.0.1:LOCAL:127.0.0.1:REMOTE user@server
```

## Connection health

After starting a tunnel, Jean polls `/api/auth?token=...`. A connection is
accepted only when token validation succeeds and the remote Jean version matches
the desktop backend version. Tunnel status is runtime-only; persisted server
records are normalized to disconnected when loaded.

## Client transport routing

`src/lib/transport.ts` owns a registry of remote `WsTransport` instances keyed
by server ID. Native calls without a backend handle continue to use Tauri IPC.
Calls carrying `_backendHandle` use the corresponding remote WebSocket.

Remote event payloads include their backend origin. Project and chat services
use that origin, persisted project clone metadata, and worktree-to-server
mappings to prevent remote updates from being applied to local cache entries.
Transport registration completes only after the WebSocket opens; callers do
not rely on fixed startup delays.

## Project and session lifecycle

Projects can be cloned onto a connected server. SSH-style git URLs are resolved
through the local SSH config, the selected identity is loaded into the local
agent, and agent forwarding lets the remote git process authenticate without
copying the private key to the server.

New worktrees can target local execution or a connected remote server. Remote
worktrees retain their server ownership in query caches and persisted UI state.
Session reads and mutations derive the backend from that ownership, including
message send, close, archive, cancel, and resume operations.

Claude CLI installation and authentication are queried independently on each
server. The login terminal is created on the selected remote backend. Other
backend-specific authentication flows still require explicit validation.

## Startup and recovery

Provisioned servers auto-connect when Jean starts. The SSH tunnel is opened
first, then its WebSocket transport is registered. Once ready, remote server and
worktree queries are invalidated so existing worktrees become visible without
being recreated. Failed registrations disconnect the partial tunnel.

The WebSocket client tracks per-session sequence numbers for replay after a
socket reconnect. Runtime status polling detects an exited SSH child, recreates
the tunnel, and updates the existing transport to its new local port so replay
state is retained.

Run `bun run test:remote-tunnel` for a real SSH transport test using Docker. It
starts an ephemeral `sshd` and sequenced WebSocket backend, kills the local
forward, creates a replacement tunnel on another port, and verifies replay of
the missed event. This covers tunnel and transport recovery without requiring a
cloud server. It does not validate systemd, AppImage provisioning, or Xvfb.

Run `bun run test:remote-provision` on macOS for the full provisioning path
using a Lima Linux VM. The test invokes Jean's real `add_remote_server`,
`test_remote_server`, `provision_remote_server`, and `connect_remote_server`
commands from an isolated local profile. It verifies the signed release
download, AppImage installation, Xvfb and WebKitGTK packages, the enabled and
running systemd service, authenticated tunnel health, and remote WebSocket
dispatch. Install Lima with `brew install lima` first and build the local debug
Jean binary. The test reuses an existing `jean-remote-provision-test` VM when
present; otherwise it creates an ephemeral VM and deletes it afterward.

## Commands

- `add_remote_server`
- `update_remote_server`
- `remove_remote_server`
- `list_remote_servers`
- `test_remote_server`
- `list_remote_jean_versions`
- `provision_remote_server`
- `connect_remote_server`
- `disconnect_remote_server`
- `get_remote_server_status`

Mutating WebSocket dispatch arms emit cache invalidations for
`remote-servers` and, when persisted data changes, `preferences`.

## Remaining constraints

- Linux provisioning requires systemd and a supported package manager.
- Xvfb is required until the server runtime is decoupled from Tauri's GTK
  initialization.
- SSH passwords remain in local preferences; SSH key authentication is
  recommended.
- Reverse port-forwarding UI is not implemented.
- Remote self-update/reprovisioning and multi-user server management are out of
  scope for the MVP.
