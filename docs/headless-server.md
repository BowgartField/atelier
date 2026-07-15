# Jean Headless Server

Jean provides two browser-accessible server entrypoints:

- `jean-server` is the standalone true-headless binary. It does not link Tauri,
  Wry, GTK, WebKitGTK, or other graphical libraries and does not need a display.
- `jean --headless` is the compatibility runtime. It still initializes Tauri
  and, on Linux, needs GTK/WebKitGTK plus an X/Wayland display or `xvfb-run`.

The standalone server command surface is being migrated capability by
capability. Keep the compatibility runtime for workflows not yet listed as
supported by the standalone binary.

Clients can call `get_server_capabilities` over WebSocket to obtain the exact
registry. Available commands are reported as `core`; every command present in
the desktop WebSocket dispatcher but not yet extracted is reported explicitly
as unavailable (`adapter_backed` or `desktop_only`). Invoking one returns the
stable `unsupported` error code rather than an accidental `Unknown command`.
The generated registry is checked by `bun run test:server-ci`, so adding a
desktop WebSocket command cannot silently leave the headless inventory stale.

The current standalone core covers persistence and preferences, project and
worktree CRUD, local Git diff/history/commit/pull/push/stash/revert operations,
session lifecycle, all seven AI CLI entrypoints, PTY streaming/replay, and
cancellation. GitHub/Linear workflows and desktop integrations remain visible
in the capability registry until their adapters are extracted.

## Start locally

When running a debug binary directly with `cargo build` / `./target/debug/jean-server`,
build the browser bundle first. Jean embeds `dist/` into the server binary at
compile time, so production deploys only need the compiled binary.

```bash
bun run build
cargo build -p jean-server
```

```bash
env -u DISPLAY -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE \
  ./target/debug/jean-server --host 127.0.0.1 --port 3456
curl http://127.0.0.1:3456/healthz
```

You can also run the server entrypoint when packaged/available:

```bash
jean-server --host 127.0.0.1 --port 3456
```

For a production single-binary server:

```bash
bun run build
cargo build --locked --release -p jean-server
./target/release/jean-server --host 0.0.0.0 --port 3456 --token "$JEAN_TOKEN"
```

After `cargo build --release -p jean-server` finishes, `dist/` is no longer
needed on the target server. Re-run `bun run build` before compiling whenever
frontend code changes.

To run the compatibility runtime instead:

```bash
xvfb-run -a ./target/debug/jean --headless --host 127.0.0.1 --port 3456
```

## Options and environment

| CLI | Environment | Default |
| --- | --- | --- |
| `--host <addr>` | `JEAN_HOST` | `127.0.0.1` |
| `--port <port>` | `JEAN_PORT` | `3456` |
| `--token <token>` | `JEAN_TOKEN` | generated token |
| `--no-token` | `JEAN_NO_TOKEN=1` | off |
| `--data-dir <path>` | `JEAN_DATA_DIR` | platform app-data directory |

By default a token is required (using `--token`, `JEAN_TOKEN`, or an
auto-generated one). `jean-server` rejects `--no-token` on a non-loopback bind.

## Health checks

- `GET /healthz` — process is alive.
- `GET /readyz` — HTTP server is initialized and WebSocket broadcaster state is ready.

Authenticated endpoints accept either the existing `?token=...` query parameter or an HTTP bearer token:

```bash
curl -H "Authorization: Bearer $JEAN_TOKEN" http://127.0.0.1:3456/api/auth
curl "http://127.0.0.1:3456/api/init?token=$JEAN_TOKEN"
```

The browser UI still uses `/api/init`, `/api/auth`, and `/ws` from the same origin, so reverse proxies do not need to rewrite paths.

## systemd example

```ini
[Unit]
Description=Jean headless server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=jean
Environment=JEAN_HOST=127.0.0.1
Environment=JEAN_PORT=3456
Environment=JEAN_TOKEN=change-me-long-random-token
ExecStart=/usr/local/bin/jean-server
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## Docker notes

- The server Docker image is published by the Server Release workflow as
  `ghcr.io/<owner>/<repo>-server:<tag>`.
- The image starts `jean-server` directly and contains no Xvfb/GTK/WebKitGTK runtime packages.
- Bind to `0.0.0.0` inside the container, but keep token auth enabled.
- Mount Jean's app-data directory as a volume so projects, preferences, and sessions persist.
- Put TLS/auth in front of the container for internet exposure.

Example command:

```bash
docker run --rm \
  -e JEAN_HOST=0.0.0.0 \
  -e JEAN_PORT=3456 \
  -e JEAN_TOKEN=change-me-long-random-token \
  -p 127.0.0.1:3456:3456 \
  -v jean-data:/home/jean/.local/share/com.jean.desktop \
  ghcr.io/OWNER/REPO-server:latest
```

## Reverse proxy

### Caddy

```caddyfile
jean.example.com {
  encode zstd gzip
  reverse_proxy 127.0.0.1:3456
}
```

### Nginx

```nginx
server {
  listen 443 ssl http2;
  server_name jean.example.com;

  location / {
    proxy_pass http://127.0.0.1:3456;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }
}
```

## Tailscale binding

Bind directly to the Tailscale IP and keep token auth enabled:

```bash
jean-server --host 100.x.y.z --port 3456 --token "$JEAN_TOKEN"
```

## Security recommendations

- Prefer `127.0.0.1` behind Caddy/Nginx, SSH tunnel, or Tailscale.
- Keep token auth enabled for every non-localhost bind.
- Use a long random token, for example `openssl rand -base64 32`.
- Set `JEAN_ALLOWED_ORIGINS=https://jean.example.com` only when you need cross-origin browser access; otherwise keep the default same-origin behavior.
