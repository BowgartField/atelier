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
a vertical step timeline on the left, and live logs on the right. The active
step uses a spinning marker; completed and active steps share the same accent
color so progress reads as one continuous path.

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

Adding a server immediately verifies SSH and reports `connecting`, `connected`,
or `error` without requiring a separate Test SSH click. A successful SSH test
also remains `connected`; it no longer resets the card to `disconnected`.
Every 30 seconds the app checks SSH reachability for unprovisioned servers and
the authenticated API for provisioned servers.

When SSH points to an already-provisioned Jean host, the add flow reads the
installed version and service token from the root-owned installation metadata.
The frontend then opens the authenticated tunnel and discovers the projects
registered on that backend.

After starting a tunnel, Jean polls `/api/auth?token=...`. A backend connection
is accepted only when token validation succeeds and the remote Jean version
matches the desktop backend version. Tunnel status is runtime-only; persisted
server records are normalized to disconnected when loaded.

## Client transport routing

`src/lib/transport.ts` owns a registry of remote `WsTransport` instances keyed
by server ID. Native calls without a backend handle continue to use Tauri IPC.
Calls carrying `_backendHandle` use the corresponding remote WebSocket.

Remote event payloads include their backend origin. Project and chat services
use that origin, persisted project clone metadata, and worktree-to-server
mappings to prevent remote updates from being applied to local cache entries.
Transport registration completes only after the WebSocket opens; callers do
not rely on fixed startup delays. Once registered, the project query combines
local projects with the connected backend's registered projects and tags remote
entries with their server ID for subsequent command routing.

## Project and session lifecycle

Projects can be cloned onto a connected server. SSH-style git URLs are resolved
through the local SSH config, the selected identity is loaded into the local
agent, and agent forwarding lets the remote git process authenticate without
copying the private key to the server.

New worktrees can target local execution or a connected remote server. Remote
worktrees retain their server ownership in query caches and persisted UI state.
Session reads and mutations derive the backend from that ownership, including
message send, close, archive, cancel, and resume operations.

AI CLI installation, authentication, and available versions are queried
independently on each connected server. The server card shows installed CLIs as
a horizontal shelf of square cards, with a separate picker for installing
additional backends. Outdated cards open the shared CLI update modal with the
remote backend handle, so the selected installer runs on that server rather
than locally. Cursor keeps its native terminal-based install/update flow, also
routed through the remote backend. Login terminals are created on the selected
server for every supported AI CLI.

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

## Testing the PR manually with Lima

The automated `test:remote-provision` script drives the whole flow headlessly.
To exercise the real UI end to end against a local Linux VM, use the Lima VM but
provision and connect from the running app instead.

Prerequisites (macOS):

- `brew install lima`
- A running Jean dev app (`bun run tauri:dev`), or the built debug binary.

Steps:

1. **Boot the VM.** Run `bun run test:remote-provision:spinup`. It creates (or
   reuses) the `jean-remote-provision-test` Lima VM and prints the SSH
   coordinates it leaves running:

   ```text
   Host:          127.0.0.1
   Port:          <sshLocalPort>
   User:          <your lima user>
   Identity file: <path to lima id_ed25519>
   ```

2. **Add the server.** In the app open Preferences → Remote Servers → Add
   Server. Enter the printed host, port, and user, choose SSH key auth, and
   point it at the printed identity file. Adding it runs an SSH test; the card
   should move to `connecting`/`connected` and report the detected OS and
   architecture.

3. **Provision.** Use the server card's provision action and pick a Jean
   version matching your desktop build (omit to use the desktop version). The
   provisioning modal shows the step timeline and live logs: package install,
   signed AppImage download, `scp` upload, and `jean-remote.service` install.
   It reports success only after `/api/auth` responds.

4. **Connect.** Once provisioned the app opens the SSH tunnel, registers the
   remote WebSocket transport, and discovers projects registered on that
   backend. The server card shows `connected` and its installed AI CLIs.

5. **Clone a repo.** Clone a project onto the connected server. Use an
   SSH-style git URL that resolves through your local SSH config — agent
   forwarding lets the remote git authenticate without copying your key. The
   cloned project appears tagged with the server ID.

6. **Create a remote worktree + session.** Add a new worktree targeting the
   connected server, open a session, and send a message. Verify the session
   runs on the remote backend (send, cancel, close, archive, resume all route
   to the server) and that streaming output arrives over the tunnel.

7. **Recovery (optional).** Kill the SSH child (`pkill -f "ssh -N -L"`) or
   restart the app; the tunnel should be recreated on a new local port and the
   remote worktrees should reappear without being recreated.

8. **Clean up.** Remove the server in the UI, then delete the VM:
   `limactl delete --force jean-remote-provision-test`.

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
- `check_remote_server_health`

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
