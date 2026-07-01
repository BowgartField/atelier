#!/usr/bin/env node
/* global AbortSignal, WebSocket, fetch */

import { execFileSync, spawn } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath, URL } from 'node:url'

const root = resolve(fileURLToPath(new URL('..', import.meta.url)))
const spinupOnly =
  process.argv.includes('--spinup-only') ||
  process.env.JEAN_REMOTE_TEST_SPINUP_ONLY === '1'
const instanceName =
  process.env.JEAN_REMOTE_TEST_VM ?? 'jean-remote-provision-test'
const jeanBinary = resolve(
  root,
  process.env.JEAN_REMOTE_TEST_BINARY ?? 'src-tauri/target/debug/jean'
)
const localToken = `local-provision-${process.pid}`
const temporaryHome = mkdtempSync(
  resolve(tmpdir(), 'jean-remote-provision-home-')
)

let createdInstance = false
let startedExistingInstance = false
let jeanProcess
let localClient
let remoteClient
let serverId

function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: root,
    encoding: 'utf8',
    stdio: options.inherit ? 'inherit' : ['ignore', 'pipe', 'pipe'],
    env: options.env ?? process.env,
  })
}

function commandExists(command) {
  try {
    run('/usr/bin/which', [command])
    return true
  } catch {
    return false
  }
}

function getInstance() {
  try {
    const output = run('limactl', [
      'list',
      '--format',
      'json',
      instanceName,
    ]).trim()
    return output ? JSON.parse(output) : null
  } catch {
    return null
  }
}

function ensureInstance() {
  let instance = getInstance()
  if (!instance) {
    console.log(`Creating Lima VM ${instanceName}...`)
    run(
      'limactl',
      [
        'start',
        `--name=${instanceName}`,
        '--vm-type=vz',
        '--cpus=4',
        '--memory=8',
        '--disk=30',
        '--containerd=none',
        '--mount-none',
        '--tty=false',
        '--timeout=15m',
        'template:default',
      ],
      { inherit: true }
    )
    createdInstance = true
    instance = getInstance()
  } else if (instance.status !== 'Running') {
    console.log(`Starting Lima VM ${instanceName}...`)
    run('limactl', ['start', '--tty=false', '--timeout=15m', instanceName], {
      inherit: true,
    })
    startedExistingInstance = true
    instance = getInstance()
  }

  if (!instance || instance.status !== 'Running') {
    throw new Error(`Lima VM ${instanceName} did not become ready`)
  }
  return instance
}

async function reservePort() {
  return new Promise((resolvePort, reject) => {
    const server = createServer()
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close(error => {
        if (error || port === null) reject(error ?? new Error('No free port'))
        else resolvePort(port)
      })
    })
  })
}

function startLocalJean(port) {
  if (!existsSync(jeanBinary)) {
    throw new Error(
      `Jean debug binary not found at ${jeanBinary}. Build it before running this test.`
    )
  }
  const environment = {
    ...process.env,
    HOME: temporaryHome,
    XDG_CONFIG_HOME: resolve(temporaryHome, '.config'),
    XDG_DATA_HOME: resolve(temporaryHome, '.local/share'),
  }
  const child = spawn(
    jeanBinary,
    [
      '--headless',
      '--host',
      '127.0.0.1',
      '--port',
      String(port),
      '--token',
      localToken,
    ],
    { cwd: root, env: environment, stdio: ['ignore', 'ignore', 'pipe'] }
  )
  child.stderrText = ''
  child.stderr.on('data', data => {
    child.stderrText += data.toString()
  })
  return child
}

async function waitForHealth(url, token, child) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(
        `${url}/api/auth?token=${encodeURIComponent(token)}`,
        { signal: AbortSignal.timeout(2_000) }
      )
      if (response.ok) return
    } catch {
      // The server or tunnel is not ready yet.
    }
    if (child?.exitCode !== null) {
      throw new Error(`Jean exited early: ${child.stderrText.trim()}`)
    }
    await new Promise(resolveWait => setTimeout(resolveWait, 200))
  }
  throw new Error(`Timed out waiting for ${url}`)
}

function createClient(url, onEvent) {
  return new Promise((resolveClient, reject) => {
    const socket = new WebSocket(url)
    const pending = new Map()
    let nextId = 0

    socket.addEventListener(
      'error',
      () => reject(new Error('WebSocket connection failed')),
      { once: true }
    )
    socket.addEventListener(
      'open',
      () => {
        resolveClient({
          invoke(command, args = {}, timeoutMs = 15 * 60_000) {
            return new Promise((resolveInvoke, rejectInvoke) => {
              const id = `lima-${++nextId}`
              const timeout = setTimeout(() => {
                pending.delete(id)
                rejectInvoke(new Error(`${command} timed out`))
              }, timeoutMs)
              pending.set(id, {
                resolve: value => {
                  clearTimeout(timeout)
                  resolveInvoke(value)
                },
                reject: error => {
                  clearTimeout(timeout)
                  rejectInvoke(error)
                },
              })
              socket.send(JSON.stringify({ type: 'invoke', id, command, args }))
            })
          },
          close() {
            socket.close()
          },
        })
      },
      { once: true }
    )
    socket.addEventListener('message', event => {
      const message = JSON.parse(event.data)
      if (message.type === 'event') {
        onEvent?.(message)
        return
      }
      const request = message.id ? pending.get(message.id) : null
      if (!request) return
      pending.delete(message.id)
      if (message.type === 'error') {
        request.reject(new Error(message.error))
      } else {
        request.resolve(message.data)
      }
    })
  })
}

function verifyRemoteSystem(version) {
  const script = `
set -euo pipefail
systemctl is-active --quiet jean-remote.service
systemctl is-enabled --quiet jean-remote.service
test -x /opt/jean-remote/jean.AppImage
test "$(cat /opt/jean-remote/VERSION)" = "$1"
command -v xvfb-run >/dev/null
dpkg-query -W -f='\${Status}\\n' libwebkit2gtk-4.1-0 |
  grep -q 'install ok installed'
test "$(systemctl show jean-remote.service -p SubState --value)" = "running"
test "$(systemctl show jean-remote.service -p ExecMainStatus --value)" = "0"
`
  run('limactl', [
    'shell',
    instanceName,
    '--',
    'bash',
    '-c',
    script,
    'verify-jean',
    version,
  ])
}

async function stopChild(child) {
  if (!child || child.exitCode !== null) return
  child.kill('SIGTERM')
  await Promise.race([
    new Promise(resolveExit => child.once('exit', resolveExit)),
    new Promise(resolveTimeout =>
      setTimeout(() => {
        child.kill('SIGKILL')
        resolveTimeout()
      }, 5_000)
    ),
  ])
}

async function cleanup() {
  remoteClient?.close()
  if (localClient && serverId) {
    try {
      await localClient.invoke('disconnect_remote_server', { serverId }, 10_000)
    } catch {
      // Process shutdown also terminates tracked tunnels.
    }
  }
  localClient?.close()
  await stopChild(jeanProcess)
  rmSync(temporaryHome, { recursive: true, force: true })

  if (spinupOnly) {
    // Leave the VM running so it can be provisioned from the Jean UI.
    return
  }

  if (createdInstance) {
    run('limactl', ['delete', '--force', instanceName], { inherit: true })
  } else if (startedExistingInstance) {
    run('limactl', ['stop', instanceName], { inherit: true })
  }
}

async function main() {
  if (process.platform !== 'darwin') {
    throw new Error('This integration test currently requires macOS and Lima')
  }
  if (!commandExists('limactl')) {
    throw new Error('Lima is required. Install it with: brew install lima')
  }

  const instance = ensureInstance()

  if (spinupOnly) {
    console.log(`Lima VM ${instanceName} is up.`)
    console.log('Add it as a remote server in the Jean UI and provision from there:')
    console.log(`  Host:          ${instance.sshAddress}`)
    console.log(`  Port:          ${instance.sshLocalPort}`)
    console.log(`  User:          ${instance.config.user.name}`)
    console.log(`  Identity file: ${instance.IdentityFile}`)
    console.log(`When done, delete it with: limactl delete --force ${instanceName}`)
    return
  }

  const localPort = await reservePort()
  jeanProcess = startLocalJean(localPort)
  const localUrl = `http://127.0.0.1:${localPort}`
  await waitForHealth(localUrl, localToken, jeanProcess)

  localClient = await createClient(
    `${localUrl.replace(/^http/, 'ws')}/ws?token=${localToken}`,
    event => {
      if (event.event !== 'remote-server:provision-progress') return
      const { percent, stage, message } = event.payload
      console.log(`[${percent}%] ${stage}: ${message}`)
    }
  )

  const existing = await localClient.invoke('list_remote_servers')
  if (existing.length !== 0) {
    throw new Error('The isolated Jean profile unexpectedly contains servers')
  }

  const server = await localClient.invoke('add_remote_server', {
    config: {
      name: 'Lima provisioning test',
      host: instance.sshAddress,
      port: instance.sshLocalPort,
      username: instance.config.user.name,
      auth: {
        type: 'ssh_key_path',
        path: instance.IdentityFile,
      },
      default: true,
      remote_port: 3456,
    },
  })
  serverId = server.id

  const connectionTest = await localClient.invoke('test_remote_server', {
    serverId,
  })
  if (!connectionTest.success) throw new Error(connectionTest.message)
  console.log(
    `SSH verified (${connectionTest.os}, ${connectionTest.architecture})`
  )

  const provisioned = await localClient.invoke('provision_remote_server', {
    serverId,
  })
  verifyRemoteSystem(provisioned.version)
  console.log('AppImage, Xvfb, WebKitGTK, and systemd verified')

  const connection = await localClient.invoke('connect_remote_server', {
    serverId,
  })
  await waitForHealth(connection.url, connection.token)
  remoteClient = await createClient(
    `${connection.url.replace(/^http/, 'ws')}/ws?token=${encodeURIComponent(connection.token)}`
  )
  const preferences = await remoteClient.invoke('load_preferences')
  if (!preferences || typeof preferences !== 'object') {
    throw new Error('Remote WebSocket returned invalid preferences')
  }

  console.log('Authenticated tunnel and remote WebSocket verified')
  console.log('Remote provisioning integration test passed.')
}

main()
  .catch(error => {
    console.error(error.message)
    process.exitCode = 1
  })
  .finally(cleanup)
