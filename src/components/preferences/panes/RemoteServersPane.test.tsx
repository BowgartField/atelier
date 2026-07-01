import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { RemoteServerConfig } from '@/types/remote'
import { useUIStore } from '@/store/ui-store'
import { RemoteServersPane } from './RemoteServersPane'

const mocks = vi.hoisted(() => ({
  servers: [] as RemoteServerConfig[],
  add: vi.fn(),
  update: vi.fn(),
  remove: vi.fn(),
  test: vi.fn(),
  provision: vi.fn(),
  connect: vi.fn(),
  disconnect: vi.fn(),
  refetch: vi.fn(),
  claudeInstalled: true,
  claudeAuthenticated: false,
  invoke: vi.fn(),
}))

vi.mock('@/services/claude-cli', () => ({
  useClaudeCliStatus: () => ({
    data: {
      installed: mocks.claudeInstalled,
      version: '2.1.196',
      path: '/opt/jean/bin/claude',
      supports_auth_command: true,
    },
    isLoading: false,
  }),
  useClaudeCliAuth: () => ({
    data: { authenticated: mocks.claudeAuthenticated, error: null },
    isLoading: false,
  }),
}))

vi.mock('@/services/remote-servers', () => ({
  useRemoteServers: () => ({
    data: mocks.servers,
    isLoading: false,
    isFetching: false,
    isError: false,
    error: null,
    refetch: mocks.refetch,
  }),
  useAddRemoteServer: () => ({
    mutateAsync: mocks.add,
    isPending: false,
  }),
  useUpdateRemoteServer: () => ({
    mutateAsync: mocks.update,
    isPending: false,
  }),
  useRemoveRemoteServer: () => ({ mutateAsync: mocks.remove }),
  useTestRemoteServer: () => ({ mutateAsync: mocks.test }),
  useProvisionRemoteServer: () => ({ mutateAsync: mocks.provision }),
  useRemoteJeanVersions: () => ({ data: [], isLoading: false }),
  useConnectRemoteServer: () => ({ mutateAsync: mocks.connect }),
  useDisconnectRemoteServer: () => ({ mutateAsync: mocks.disconnect }),
}))

vi.mock('@/lib/transport', () => ({
  listen: vi.fn(async () => vi.fn()),
  invoke: mocks.invoke,
}))

vi.mock('@/lib/platform', () => ({
  isMacOS: true,
}))

vi.mock('sonner', () => ({
  toast: {
    loading: vi.fn(() => 'toast-id'),
    success: vi.fn(),
    error: vi.fn(),
  },
}))

const server: RemoteServerConfig = {
  id: 'server-1',
  name: 'Test server',
  host: '203.0.113.10',
  port: 22,
  username: 'test-user',
  auth: {
    type: 'ssh_key_path',
    path: '~/.ssh/id_test',
    passphrase: 'test-passphrase',
  },
  default: true,
  remote_port: 3456,
  status: 'disconnected',
  http_token: 'token',
  installed_version: '0.1.60',
}

describe('RemoteServersPane', () => {
  beforeEach(() => {
    mocks.servers = []
    Object.values(mocks).forEach(value => {
      if (typeof value === 'function' && 'mockReset' in value) {
        value.mockReset()
      }
    })
    mocks.add.mockResolvedValue(server)
    mocks.claudeInstalled = true
    mocks.claudeAuthenticated = false
    mocks.invoke.mockResolvedValue({ claude_cli: false, gh_cli: false })
    useUIStore.getState().closeCliLoginModal()
    mocks.test.mockResolvedValue({
      success: true,
      message: 'SSH connection successful',
      hostname: 'example-remote-host',
      os: 'Linux',
      architecture: 'x86_64',
    })
  })

  it('adds a key-authenticated remote server from the empty state', async () => {
    const user = userEvent.setup()
    render(<RemoteServersPane />)

    await user.click(
      screen.getByRole('button', { name: 'Add your first server' })
    )
    await user.type(screen.getByLabelText('Display name'), 'Test server')
    await user.type(screen.getByLabelText('Host or IP address'), '203.0.113.10')
    await user.clear(screen.getByLabelText('SSH username'))
    await user.type(screen.getByLabelText('SSH username'), 'test-user')
    await user.clear(screen.getByLabelText('Private key path'))
    await user.type(screen.getByLabelText('Private key path'), '~/.ssh/id_test')
    await user.type(screen.getByLabelText(/Key passphrase/), 'test-passphrase')
    await user.click(screen.getByRole('button', { name: 'Add server' }))

    await waitFor(() => {
      expect(mocks.add).toHaveBeenCalledWith({
        name: 'Test server',
        host: '203.0.113.10',
        port: 22,
        username: 'test-user',
        auth: {
          type: 'ssh_key_path',
          path: '~/.ssh/id_test',
          passphrase: 'test-passphrase',
        },
        default: false,
        remote_port: 3456,
      })
    })
  })

  it('tests SSH for a configured server', async () => {
    mocks.servers = [server]
    const user = userEvent.setup()
    render(<RemoteServersPane />)

    expect(screen.getByText('Test server')).toBeInTheDocument()
    expect(screen.getByText('0.1.60')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Test SSH' }))

    await waitFor(() => {
      expect(mocks.test).toHaveBeenCalledWith('server-1')
    })
  })

  it('opens the provisioning modal and starts provisioning', async () => {
    mocks.servers = [{ ...server, http_token: null, installed_version: null }]
    mocks.provision.mockResolvedValue({
      success: true,
      version: '0.1.60',
      remote_port: 3456,
      service_name: 'jean-remote.service',
    })
    const user = userEvent.setup()
    render(<RemoteServersPane />)

    await user.click(screen.getByRole('button', { name: 'Provision' }))
    expect(
      screen.getByRole('heading', {
        name: 'Provision Test server',
      })
    ).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Provision server' }))

    await waitFor(() => {
      expect(mocks.provision).toHaveBeenCalledWith(
        expect.objectContaining({ serverId: 'server-1' })
      )
    })
  })

  it('detects missing Claude auth and opens login on the remote backend', async () => {
    mocks.servers = [{ ...server, status: 'connected' }]
    const user = userEvent.setup()
    render(<RemoteServersPane />)

    expect(screen.getByText('Login required')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Login' }))

    expect(useUIStore.getState()).toMatchObject({
      cliLoginModalOpen: true,
      cliLoginModalType: 'claude',
      cliLoginModalCommand: '/opt/jean/bin/claude',
      cliLoginModalCommandArgs: ['auth', 'login'],
      cliLoginModalBackendHandle: 'server-1',
    })
  })

  it('shows when Claude is authenticated on the remote backend', () => {
    mocks.servers = [{ ...server, status: 'connected' }]
    mocks.claudeAuthenticated = true

    render(<RemoteServersPane />)

    expect(screen.getByText('Logged in')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Login' })).toBeNull()
  })
})
