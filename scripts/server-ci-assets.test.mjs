import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const read = path => readFileSync(path, 'utf8')

test('server release workflow builds binaries and publishes docker image', () => {
  const workflow = read('.github/workflows/server-release.yml')

  assert.match(workflow, /name: Verify CI build/)
  assert.match(workflow, /workflow_id: 'ci-build\.yml'/)
  assert.match(workflow, /head_sha: commit\.sha/)
  assert.match(workflow, /needs: \[metadata, verify-ci\]/)
  assert.match(workflow, /cd src-server && cargo build --release/)
  assert.match(workflow, /jean-server-linux-amd64/)
  assert.match(workflow, /jean-server-linux-arm64/)
  assert.match(workflow, /docker\/build-push-action@v6/)
  assert.match(workflow, /ghcr\.io/)
})

test('Dockerfile builds and runs jean-server headlessly as non-root user', () => {
  const dockerfile = read('Dockerfile.server')

  assert.match(dockerfile, /bun run build/)
  assert.match(dockerfile, /COPY src-server \.\/src-server/)
  assert.match(
    dockerfile,
    /cd src-server && RUSTC_WRAPPER= cargo build --release/
  )
  assert.match(dockerfile, /USER jean/)
  assert.match(dockerfile, /chown -R jean:jean \/home\/jean/)
  assert.match(dockerfile, /JEAN_HOST=0\.0\.0\.0/)
  assert.match(dockerfile, /xvfb/)
  assert.match(dockerfile, /jean-server-entrypoint/)
  assert.match(
    dockerfile,
    /ENTRYPOINT \["\/usr\/local\/bin\/jean-server-entrypoint"\]/
  )
})

test('jean-server is outside the desktop Tauri package so macOS universal bundling ignores it', () => {
  const cargoToml = read('src-tauri/Cargo.toml')
  const serverCargoToml = read('src-server/Cargo.toml')

  assert.doesNotMatch(cargoToml, /jean-server/)
  assert.match(serverCargoToml, /name = "jean-server"/)
  assert.match(
    serverCargoToml,
    /jean_lib = \{ package = "jean", path = "\.\.\/src-tauri" \}/
  )
})

test('Docker entrypoint starts Xvfb before jean-server', () => {
  const entrypoint = read('scripts/docker-entrypoint.sh')

  assert.match(entrypoint, /Xvfb :99/)
  assert.match(entrypoint, /export DISPLAY=:99/)
  assert.match(entrypoint, /sleep 0\.5/)
  assert.match(entrypoint, /exec jean-server "\$@"/)
})
