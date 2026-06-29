# Remote Servers

Remote servers let the desktop app provision and connect to a headless Jean
backend through an SSH local-forward. Local projects continue to use Tauri IPC;
remote transport routing is introduced separately.

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
artifact from the release manifest matching the desktop's exact version,
verifies its updater minisign signature with the same public key as the desktop
updater, uploads the extracted AppImage with `scp`, and installs
`jean-remote.service`.

`provision::jean_launch_command()` is the single Xvfb compatibility boundary:

```text
xvfb-run -a jean.AppImage --headless --host 127.0.0.1 --port P --token T
```

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

## Commands

- `add_remote_server`
- `update_remote_server`
- `remove_remote_server`
- `list_remote_servers`
- `test_remote_server`
- `provision_remote_server`
- `connect_remote_server`
- `disconnect_remote_server`
- `get_remote_server_status`

Mutating WebSocket dispatch arms emit cache invalidations for
`remote-servers` and, when persisted data changes, `preferences`.
