# Remote Servers — run Jean sessions on a cloud box over SSH

## Context

Today Jean runs all AI sessions (Claude/Codex/Pi/Cursor/OpenCode CLIs), terminals, git, and file ops **locally** on the machine where the desktop app runs. The goal: let the user register **remote servers** (IP, port, SSH key/password), and run sessions that actually execute **on that server** — terminals, git, and the AI CLIs all run remote — while **local and remote sessions coexist in the same window** (decision 1b).

Key enabling fact discovered during exploration: Jean **already** ships a headless server mode (`jean --headless --host --port --token`, `lib.rs:3433`+, `start_http_server_headless`) that exposes the entire command surface over HTTP+WebSocket (`http_server/dispatch.rs`), and the frontend (`src/lib/transport.ts`) **already** has a `WsTransport` client used for "web access". So a remote session = a headless Jean on the cloud box, reached through an **SSH tunnel**, driven by a second `WsTransport`. PTY terminals, detached Claude, the Pi RPC socket, and Codex stdio all "just work" remotely because they are *local* from the remote backend's perspective.

This is large. Plan is phased; Phase 1 alone is shippable value (one remote box, attach, terminal + chat working).

## Decisions (locked)
- **1b** Mixed local + remote sessions in one window.
- **2a** SSH via the **system `ssh`/`scp`** binary (ControlMaster, `-L`/`-R`), not a Rust SSH crate.
- **3b** Jean **auto-provisions** the server: check/install OpenSSH server, install Jean as a service, start it headless. Jean does **not** manage server OS updates.
- **4a** AI CLIs authenticate **on the server** (user runs `claude login` etc. via the remote terminal).

## Critical feasibility constraint (DECIDED: xvfb)
`jean --headless` still calls Tauri `.build()`, which initializes GTK/WebKitGTK on Linux (`Cargo.toml` pins `webkit2gtk`/`gtk`) — it needs a display even though the window closes immediately. On a display-less cloud box it will fail. **Decision: install `xvfb` on the server and run the service under `xvfb-run -a`.** This is not user-visible and reuses the existing `--headless` mode with ~zero backend changes.

**Forward-compat:** Jean upstream is improving headless mode (may drop the GTK/display dependency). Keep the service launch command behind a **single config point** (`provision::jean_launch_command()`) returning the full exec line, so when a true GUI-free headless lands, removing `xvfb-run` (and the xvfb install step) is a one-line change with no other code touched. The GUI-free server binary (decouple from `tauri::AppHandle` across 39 files / 359 commands / 218 emits) stays out of scope — upstream headless work is expected to make it unnecessary.

---

## Architecture

```
Desktop Jean (client)
  ├─ Local backend     → native Tauri IPC            (unchanged, default)
  └─ Remote server "S" → WsTransport over SSH tunnel  → headless Jean on S
        ssh -N -L 127.0.0.1:<localPort>:127.0.0.1:<remotePort> user@S
        (remote Jean binds 127.0.0.1 only — exposed solely through the tunnel)
```

A **remote project** is a project whose data lives on server S. Every operation on that project's worktrees/sessions/terminals/git/files routes to S's `WsTransport`. Local projects keep using native IPC. Routing granularity = **the server that owns the active project**, threaded as a backend handle.

---

## Phase 1 — Server management + provisioning + tunnel (backend foundation)

**Implementation status (2026-06-29):** backend code, persistence, IPC/WebSocket
dispatch, signed artifact provisioning, tunnel lifecycle, tests, and developer
documentation are implemented. Real Linux VM validation remains required before
Phase 1 can be considered production-verified.

New Rust module `src-tauri/src/remote/` :

- `types.rs` — `RemoteServerConfig { id, name, host, port, username, auth: SshKeyPath|Password, default, status }`. Mirror in `src/types/remote.ts` (snake_case).
- Storage: add `remote_servers: Vec<RemoteServerConfig>` to `AppPreferences` (`lib.rs`), same pattern as `custom_cli_profiles`. Secrets: store key path / reference; if password, keep it out of plain `preferences.json` where possible (note as a follow-up; MVP may store but flag it).
- `ssh.rs` — thin wrappers over `silent_command("ssh"|"scp")` (reuse `platform::process::silent_command`):
  - `test_connection(server)` — `ssh -o BatchMode ... echo ok`.
  - `exec(server, cmd)` — run a remote command, capture stdout/stderr.
  - Use a per-server **ControlMaster** socket (`-o ControlPath=<appdata>/ssh/<id>.sock -o ControlPersist`) so subsequent calls multiplex one auth'd connection.
- `provision.rs` — idempotent setup over `exec`:
  1. Detect distro / privilege (sudo).
  2. Ensure OpenSSH server present (it must be, since we're already SSH'd in — really this is "ensure sshd + xvfb + deps").
  3. Install `xvfb` + WebKitGTK runtime deps.
  4. Download Jean Linux artifact to the server. Resolve the release manifest matching the desktop's exact version, verify its updater minisign signature with the public key from `tauri.conf.json`, and extract the `.tar.gz`/AppImage with `tar`+`flate2`. Source: GitHub releases (`coollabsio/jean`). Pick artifact by remote arch (`uname -m`).
  5. Generate an auth **token**, write a **systemd unit** that runs `xvfb-run -a <jean> --headless --host 127.0.0.1 --port <P> --token <T>`; `systemctl enable --now`.
- `tunnel.rs` — manage `ssh -N -L 127.0.0.1:<localPort>:127.0.0.1:<P>` as a tracked child in a registry (mirror `terminal/registry.rs`). Optional `-R <remote>:127.0.0.1:<local>` reverse forwards for testing remote services locally. Health check: poll `http://127.0.0.1:<localPort>/...` with the token.
- Tauri commands (register in **both** `lib.rs generate_handler!` and `http_server/dispatch.rs`): `add_remote_server`, `update_remote_server`, `remove_remote_server`, `list_remote_servers`, `test_remote_server`, `provision_remote_server`, `connect_remote_server` (open tunnel, return `{localPort, token}`), `disconnect_remote_server`, `get_remote_server_status`.

Phase-1 acceptance: from a terminal you can `connect`, then hit the tunneled headless Jean (curl health) — backend only.

## Phase 2 — Client transport routing (the 1b core)

In `src/lib/transport.ts`:
- Replace the single `wsTransport` singleton with a **registry**: `getTransport(handle?: string): WsTransport` keyed by server id (`'default'` = local). Each `WsTransport` already tracks its own token/URL/reconnect/`_lastSeqBySession` — multiple instances coexist cleanly.
- `invoke<T>(command, args)`: if `args._backendHandle` set → route to that remote transport (even though `isNativeApp()` is true, remote sessions use WS, not native IPC); else current behavior.
- `listen()`: expose per-transport listeners; `useStreamingEvents` and `terminal-instances.ts` register listeners on the transport(s) of the **active remote context** in addition to local.

Backend-handle resolution (avoid editing all 269 call sites):
- Add `server_id?: string | null` to `Project` (`projects/types.rs` + `src/types/projects.ts`) — a project owned by a remote server. Default null = local.
- Add a small **active-backend context** derived from the currently-selected project's `server_id`; a thin wrapper resolves `_backendHandle` for the data hooks that operate within a project scope (chat send/stream, terminal start/write/resize/stop, git status/diff, file read, context save, PR/commit/review). These are the high-value sites (~20–30) identified in exploration; one-shot global queries (preferences, server list) stay local.
- `connect_remote_server` result feeds the matching `WsTransport` its `localPort`+token before any remote project is opened.

Phase-2 acceptance: open a remote project → its worktree list, a chat session, and a terminal all run on the server; a local session in another tab still works simultaneously.

## Phase 3 — Remote project lifecycle + polish
- Create/clone a project **on** the remote (run `git clone` via the remote backend's existing project commands — they already shell out with `current_dir`). UI to add a remote project under a server.
- Reverse-forward UI (expose a remote port locally for testing).
- Reconnect UX: tunnel drop → reconnect ssh + WS replay (the WS layer already does `last_seq` replay + terminal replay).
- Settings: `RemoteServersPane.tsx` in `src/components/preferences/panes/` (add to `PreferencesDialog` nav) — add/edit/test/provision/connect, status dots. Per-project server shown in `projects/panes/GeneralPane.tsx`.
- Surface CLI-auth state: since CLIs auth on the server (4a), guide the user to run `claude login` in the remote terminal.

---

## Key files
- New: `src-tauri/src/remote/{mod,types,ssh,provision,tunnel,commands}.rs`; `src/types/remote.ts`; `src/components/preferences/panes/RemoteServersPane.tsx`.
- Edit: `src-tauri/src/lib.rs` (AppPreferences + handler registration), `src-tauri/src/http_server/dispatch.rs` (dispatch arms), `src-tauri/src/projects/types.rs` + `src/types/projects.ts` (`server_id`), `src/lib/transport.ts` (transport registry + routing), `src/hooks/useStreamingEvents.ts`, `src/lib/terminal-instances.ts`, `src/components/preferences/PreferencesDialog.tsx`, `src/components/projects/panes/GeneralPane.tsx`.
- Reuse: `platform::process::silent_command` (ssh/scp), the Tauri updater minisign verification pattern, registry pattern from `terminal/registry.rs`, `WsTransport` from `transport.ts`.

## Out of scope (MVP)
- Server OS/Jean auto-updates (user-managed, per 3b).
- Pure-Rust SSH, SFTP file browser, multi-user servers.
- Compiling a true no-webview headless Jean (xvfb is the MVP workaround).
- Encrypted-at-rest password vault (flag as security follow-up).

## Risks
- **xvfb hard requirement** — verify a real cloud box runs `xvfb-run -a jean --headless` successfully before building UI on top.
- **Version skew** — client and remote headless Jean must speak the same dispatch protocol; pin/verify versions on connect.
- **Secret handling** — SSH password / token storage in `preferences.json` is a security concern; prefer key-based auth and OS keychain later.
- **Routing leakage** — a remote-project call that forgets its `_backendHandle` silently hits the local backend; add a dev assertion when a remote project is active.

## Verification
1. **Headless feasibility (do first):** on a throwaway Linux VM, manually `apt install xvfb`, copy a Jean AppImage, run `xvfb-run -a ./Jean.AppImage --headless --host 127.0.0.1 --port 5599 --token test`; `curl` the health endpoint with the token. If this fails, stop and revisit (Cargo headless feature).
2. **Phase 1:** add a server in DB, `test_remote_server` green, `provision_remote_server` installs + starts the systemd service (check `systemctl status`), `connect_remote_server` opens the tunnel, curl the tunneled port locally.
3. **Phase 2:** open a remote project, start a terminal → commands execute on the server (`hostname` shows the box); start a chat session → AI CLI runs remote; confirm a local session in parallel is unaffected.
4. **Phase 3:** `git clone` a repo onto the server via UI; reverse-forward a remote dev server and hit it from the local browser; kill the tunnel and confirm auto-reconnect + stream replay.
5. `bun run check:all`; add Rust tests for `provision` command-string building and `ssh` arg construction, TS tests for transport registry routing (`_backendHandle` → correct transport).
