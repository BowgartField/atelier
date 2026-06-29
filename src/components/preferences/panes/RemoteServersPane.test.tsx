import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { RemoteServerConfig } from '@/types/remote'
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
  useConnectRemoteServer: () => ({ mutateAsync: mocks.connect }),
  useDisconnectRemoteServer: () => ({ mutateAsync: mocks.disconnect }),
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
  username: 'root',
  auth: {
    type: 'ssh_key_path',
    path: '~/.ssh/id_rsa',
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
    await user.clear(screen.getByLabelText('Private key path'))
    await user.type(screen.getByLabelText('Private key path'), '~/.ssh/id_rsa')
    await user.type(screen.getByLabelText(/Key passphrase/), 'test-passphrase')
    await user.click(screen.getByRole('button', { name: 'Add server' }))

    await waitFor(() => {
      expect(mocks.add).toHaveBeenCalledWith({
        name: 'Test server',
        host: '203.0.113.10',
        port: 22,
        username: 'root',
        auth: {
          type: 'ssh_key_path',
          path: '~/.ssh/id_rsa',
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

  it('requires confirmation before provisioning', async () => {
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
        name: 'Provision Jean on Test server?',
      })
    ).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Provision server' }))

    await waitFor(() => {
      expect(mocks.provision).toHaveBeenCalledWith('server-1')
    })
  })
})
