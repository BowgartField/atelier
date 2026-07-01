# True Headless Jean Server

## Context

Jean currently has two server entrypoints:

- `jean --headless`
- `jean-server`

Both ultimately call `jean_lib::run()`, construct a `tauri::Builder`, and use
Tauri's default Wry runtime. Clearing the configured windows prevents a WebView
from being created, but it does not remove the Linux graphical runtime from the
build or process.

The current Linux dependency graph still includes:

```text
tauri
└── tauri-runtime-wry
    └── wry
        └── webkit2gtk
            └── gtk
```

Jean also declares `gtk` and `webkit2gtk` directly for its Linux desktop
file-drop integration. As a result, the current server:

- requires GTK/WebKitGTK libraries;
- initializes the Tauri Linux runtime;
- requires Xvfb on servers without a display;
- cannot be deployed as a minimal backend-only binary.

The existing mode remains useful as a compatibility mode, but it is not a true
headless runtime.

## Goal

Produce a standalone `jean-server` binary that:

- has no dependency on Tauri, Wry, GTK, GDK, WebKitGTK, JavaScriptCore, or
  libsoup;
- builds on a Linux environment without GUI development packages;
- runs with `DISPLAY` and `WAYLAND_DISPLAY` unset;
- exposes the same supported HTTP and WebSocket API as the desktop backend;
- supports projects, worktrees, chat, terminals, streaming, persistence, and
  cancellation;
- embeds the frontend assets for single-binary deployment;
- can replace the current AppImage plus Xvfb remote-server deployment.

## Non-goals

- Rewriting the React frontend.
- Replacing Axum or the existing WebSocket protocol.
- Removing Tauri from the desktop application.
- Supporting desktop-only commands from `jean-server`.
- Changing remote transport routing or the user-facing remote-server model.
- Removing the Xvfb fallback before the new binary passes production
  verification.

## Architecture Decision

Tauri must be an adapter at the desktop boundary, not the owner of Jean's
backend state.

The target dependency direction is:

```text
                         ┌────────────────────┐
                         │     jean-core      │
                         │                    │
                         │ business services  │
                         │ storage and types  │
                         │ runtime contracts  │
                         └─────────┬──────────┘
                                   │
                   ┌───────────────┴───────────────┐
                   │                               │
          ┌────────▼─────────┐           ┌─────────▼────────┐
          │   jean-desktop   │           │   jean-server    │
          │                  │           │                  │
          │ Tauri adapters   │           │ Axum + WebSocket │
          │ windows/plugins  │           │ headless runtime │
          └──────────────────┘           └──────────────────┘
```

Recommended workspace layout:

```text
crates/
├── jean-core/
│   └── src/
├── jean-server/
│   └── src/
└── jean-desktop-adapter/
    └── src/
src-tauri/
├── src/
└── Cargo.toml
```

`jean-core` and `jean-server` must not depend on `tauri` or any Tauri plugin.
The desktop crate may depend on both Tauri and `jean-core`.

## Runtime Contracts

Most current coupling comes from commands receiving `tauri::AppHandle` and
using it for unrelated responsibilities. Replace those calls with narrow,
testable contracts.

Initial contracts:

```rust
pub trait AppPaths: Send + Sync {
    fn data_dir(&self) -> Result<PathBuf, String>;
    fn config_dir(&self) -> Result<PathBuf, String>;
    fn cache_dir(&self) -> Result<PathBuf, String>;
    fn resource_dir(&self) -> Result<PathBuf, String>;
}

pub trait EventSink: Send + Sync {
    fn emit_json(&self, event: &str, payload: serde_json::Value)
        -> Result<(), String>;
}
```

The concrete runtime context should own shared backend state:

```rust
pub struct BackendContext {
    pub paths: Arc<dyn AppPaths>,
    pub events: Arc<dyn EventSink>,
    pub state: BackendState,
}
```

`BackendState` should explicitly contain registries and managers currently
stored through Tauri's `manage()` / `state()` APIs. It must not become an
untyped service locator.

Expected state includes:

- WebSocket broadcaster;
- HTTP server state;
- background task manager;
- terminal registry;
- active chat/run registries;
- remote tunnel registry where applicable;
- cancellation and shutdown coordination.

The two event implementations are:

- `DesktopEventSink`: emits to Tauri IPC and connected WebSocket clients;
- `ServerEventSink`: emits only to connected WebSocket clients.

## Command Boundary

Business functions must not be Tauri commands directly.

Use this pattern:

```rust
// jean-core
pub async fn create_project(
    context: &BackendContext,
    input: CreateProjectInput,
) -> Result<Project, String> {
    // Business logic
}

// desktop adapter
#[tauri::command]
async fn create_project_command(
    app: tauri::AppHandle,
    input: CreateProjectInput,
) -> Result<Project, String> {
    create_project(backend_context(&app)?, input).await
}

// server dispatch
"create_project" => {
    let input = from_args(args)?;
    to_value(create_project(&state.context, input).await?)
}
```

This keeps Tauri parameter extraction in the desktop adapter while allowing
the server dispatcher to call the same business function.

## Desktop-only Capabilities

Every command must be classified as one of:

1. Core: works identically on desktop and server.
2. Adapter-backed: same intent, different platform implementation.
3. Desktop-only: unavailable in headless mode.

Examples:

| Capability | Classification | Headless behavior |
| --- | --- | --- |
| Projects, git, worktrees | Core | Fully supported |
| Chat and AI CLI execution | Core | Fully supported |
| PTY terminals | Core | Fully supported |
| Preferences and UI state | Core | Fully supported |
| File dialogs | Adapter-backed | Return unsupported or use path input |
| Clipboard | Adapter-backed | Browser/client operation where possible |
| Native notifications | Desktop-only | No-op or WebSocket event |
| Window state and vibrancy | Desktop-only | Explicit unsupported response |
| Updater UI | Desktop-only | Server update handled separately |
| Open URL/file manager | Adapter-backed | Client event or unsupported |

Unsupported operations must return a stable, typed error. They must not panic or
silently report success.

## Implementation Phases

### Phase 0: Correct the Current Contract

Purpose: stop treating the compatibility runtime as completed true headless
support.

- Document `jean --headless` as compatibility headless.
- Document that it still requires GTK/WebKitGTK and a virtual display on Linux.
- Keep Xvfb provisioning unchanged.
- Add a warning at server startup when the compatibility runtime is used.
- Define the true-headless acceptance checks in CI before refactoring.

Exit criteria:

- Documentation and logs accurately describe the current behavior.
- Existing remote deployment continues to work through Xvfb.

### Phase 1: Create `jean-core` and Runtime Primitives

- Create the Cargo workspace crates.
- Add `AppPaths`, `EventSink`, `BackendContext`, and typed backend state.
- Implement desktop and headless path resolution.
- Implement the WebSocket-only server event sink.
- Move shared data types that do not depend on Tauri.
- Add compile-time dependency checks preventing Tauri imports in core/server.

Start with small vertical slices rather than moving files mechanically.

Exit criteria:

- `jean-core` builds and tests without Tauri.
- `jean-server` starts, serves embedded assets, and exposes `/healthz` and
  `/readyz` without Tauri.
- The binary starts with no display environment.

### Phase 2: Persistence and Preferences

- Move project, session, message, run-log, preferences, UI-state, and recovery
  path resolution behind `AppPaths`.
- Remove `AppHandle` from storage functions.
- Keep persisted field names and existing migration behavior unchanged.
- Test that desktop and server resolve compatible application-data layouts.
- Add fixture-based compatibility tests for existing JSON files.

Exit criteria:

- Headless server can load existing projects, preferences, sessions, and UI
  state.
- No storage module required by the server imports Tauri.

### Phase 3: Projects and Git

- Extract project CRUD business functions.
- Extract git status, diff, commit, branch, worktree, GitHub, and Linear
  operations.
- Replace direct Tauri event emission with `EventSink`.
- Wire the existing WebSocket dispatch arms to the extracted functions.
- Keep native Tauri commands as thin wrappers.

Exit criteria:

- A browser client can list, create, clone, and update projects through the true
  headless binary.
- Git and worktree operations pass the same tests in desktop and server modes.

### Phase 4: Chat and AI Backends

- Move session lifecycle, message persistence, run logs, cancellation, and
  backend execution to `jean-core`.
- Replace Tauri runtime spawning with Tokio or an injected task executor.
- Route streaming events exclusively through `EventSink`.
- Preserve backend session IDs, content block ordering, usage, tool calls, and
  recovery behavior.
- Verify Claude, Codex, Pi, Cursor, OpenCode, Command Code, and Grok according
  to their existing capability flags.

Exit criteria:

- Chat starts, streams, cancels, persists, and resumes through WebSocket.
- Detached-run recovery works after restarting `jean-server`.
- No chat module used by the server imports Tauri.

### Phase 5: Terminals and Background Tasks

- Move PTY ownership and terminal registries into `BackendState`.
- Move replay buffers and terminal sequence tracking out of Tauri-managed state.
- Refactor background polling to receive `BackendContext`.
- Add explicit shutdown coordination for terminals, AI runs, tunnels, and
  managed child processes.

Exit criteria:

- Terminal creation, input, resize, replay, reconnection, and termination work
  after browser refresh.
- Background tasks run without a GUI event loop.
- SIGTERM produces deterministic cleanup suitable for systemd and containers.

### Phase 6: Complete Dispatch Coverage

- Inventory every native `#[tauri::command]`.
- Map every server-supported command to a core function.
- Mark desktop-only commands explicitly.
- Keep cache invalidation events consistent between desktop and server.
- Add a test comparing the supported native and WebSocket command registries.

Exit criteria:

- No accidental `"Unknown command"` regressions.
- Every divergence between desktop and server is documented and tested.

### Phase 7: Release and Provisioning

- Produce a separate Linux `jean-server` artifact for `x86_64` and `aarch64`.
- Publish checksum/signature metadata.
- Update remote provisioning to prefer the true headless artifact.
- Remove GTK/WebKitGTK and Xvfb installation from the new provisioning path.
- Retain the AppImage plus Xvfb path as a rollback option for one release cycle.
- Add systemd hardening and graceful shutdown settings.

Exit criteria:

- A clean VPS can install and run `jean-server` without graphical packages.
- Remote provisioning, authentication, tunnel connection, chat, and terminal
  execution pass end-to-end tests.

### Phase 8: Remove Compatibility Debt

- Remove `jean-server`'s call to `jean_lib::run()`.
- Decide whether `jean --headless` remains as a compatibility alias or prints a
  migration message.
- Remove Xvfb provisioning after the rollback window.
- Remove obsolete headless branches from the desktop Tauri builder.
- Update architecture, release, troubleshooting, and deployment documentation.

Exit criteria:

- `jean-server` is the only supported server runtime.
- The server dependency graph is permanently GUI-free.

## Cargo Requirements

The server should be independently selectable:

```toml
[workspace]
members = [
  "crates/jean-core",
  "crates/jean-server",
  "src-tauri",
]
```

The critical rule is:

```text
jean-server ──X──> tauri
jean-core   ──X──> tauri
```

Do not rely only on:

```toml
tauri = { default-features = false }
```

Tauri itself still has unconditional Linux GTK integration. A true headless
binary cannot include the Tauri crate anywhere in its dependency graph.

## Verification

### Static Dependency Gate

Run for the Linux target:

```bash
cargo tree -p jean-server --target x86_64-unknown-linux-gnu
```

Fail CI if the result contains:

```text
tauri
tao
wry
gtk
gdk
webkit
javascriptcore
soup
pango
cairo
atk
```

### Clean Build Environment

Build in a minimal Linux container that does not install:

- GTK development packages;
- WebKitGTK development packages;
- X11 development packages;
- Xvfb.

Expected command:

```bash
cargo build --locked --release -p jean-server
```

### Linked Library Gate

Verify the produced ELF:

```bash
readelf -d target/release/jean-server
ldd target/release/jean-server
```

Neither output may reference graphical libraries.

### Runtime Gate

Run with no graphical environment:

```bash
unset DISPLAY WAYLAND_DISPLAY XDG_SESSION_TYPE
./jean-server \
  --host 127.0.0.1 \
  --port 3456 \
  --token test-token
```

Verify:

- `/healthz`;
- `/readyz`;
- `/api/auth`;
- `/api/init`;
- WebSocket authentication;
- project listing;
- session creation;
- streaming chat;
- terminal creation and replay;
- cancellation;
- graceful SIGTERM.

### Deployment Gate

Provision a clean Ubuntu and Debian VM:

- no desktop environment;
- no Xvfb;
- no GTK/WebKitGTK packages installed explicitly;
- systemd service enabled;
- browser connects through the existing SSH tunnel;
- remote `hostname` is visible from terminal and AI execution;
- service restarts cleanly after a crash.

## Test Strategy

- Unit tests for runtime contracts and path resolution.
- Contract tests executed against both desktop and headless adapters.
- Fixture tests for persisted JSON compatibility.
- Integration tests for WebSocket dispatch.
- End-to-end browser tests against the real `jean-server`.
- Linux dependency and ELF inspection gates.
- Provisioning tests for both architectures.
- Regression tests for cache invalidation and event ordering.

Tests using Tauri's mock runtime do not prove true-headless compatibility and
must not replace the real Linux runtime checks.

## Migration Safety

- Preserve the current AppImage plus Xvfb service until the new server passes
  all gates.
- Add a provisioning capability probe before selecting the new artifact.
- Keep server/client protocol version checks.
- Do not migrate stored data formats as part of the runtime split unless
  required.
- Avoid changing frontend transport behavior during the backend extraction.
- Implement one complete vertical slice at a time and keep desktop behavior
  passing throughout.

## Risks

### Large `AppHandle` Surface

The backend currently contains dozens of files and hundreds of references to
`AppHandle`. A bulk type replacement would create an untyped abstraction and
high regression risk.

Mitigation: extract by capability and introduce only narrow runtime contracts.

### Desktop Behavior Regressions

Some commands mix business logic with dialogs, clipboard, notifications, or
window operations.

Mitigation: separate business functions from adapter wrappers before moving
them.

### Event Ordering Differences

Tauri events and WebSocket broadcasts currently share some helper paths.

Mitigation: make event sequencing a core responsibility and test ordered event
streams against both adapters.

### Incomplete Command Coverage

The WebSocket dispatcher can drift from native command registration.

Mitigation: maintain a command capability registry and test its coverage.

### Release Rollback

A new server artifact changes provisioning and systemd behavior.

Mitigation: retain the signed AppImage plus Xvfb compatibility path for one
release cycle.

## Definition of Done

True headless support is complete only when all of the following are true:

- `jean-server` has no Tauri or graphical dependencies.
- It builds without GTK/WebKitGTK development packages.
- It runs without Xvfb, `DISPLAY`, or `WAYLAND_DISPLAY`.
- Core project, chat, terminal, persistence, and recovery workflows work.
- Browser refresh and WebSocket replay work.
- SIGTERM cleanup is deterministic.
- Remote provisioning installs the server artifact without GUI packages.
- Linux `x86_64` and `aarch64` artifacts pass dependency inspection.
- Desktop Jean remains functional and passes `bun run check:all`.
- Documentation no longer describes compatibility headless as true headless.

## Recommended First Implementation Slice

The first implementation PR should be deliberately narrow:

1. Create `jean-core` and `jean-server`.
2. Add `AppPaths`, `EventSink`, `BackendContext`, and headless state.
3. Move embedded asset serving, authentication, `/healthz`, and `/readyz`.
4. Start Axum directly from `jean-server`, without `tauri::Builder`.
5. Add the Linux dependency-tree and no-display runtime CI gates.
6. Keep all business commands on the existing compatibility runtime until the
   next vertical slice.

This establishes and proves the GUI-free binary boundary before migrating the
large command surface.
