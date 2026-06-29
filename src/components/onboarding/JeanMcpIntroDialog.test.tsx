import { act, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { JeanMcpIntroDialog } from './JeanMcpIntroDialog'
import { invoke } from '@/lib/transport'
import { useUIStore } from '@/store/ui-store'

const mocks = vi.hoisted(() => ({
  jeanMcpEnabled: true,
  patchPreferencesMutate: vi.fn(),
  openPreferencesPane: vi.fn(),
}))

vi.mock('@/lib/transport', () => ({
  invoke: vi.fn(),
}))

vi.mock('sonner', () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
    warning: vi.fn(),
  },
}))

vi.mock('@/services/preferences', () => ({
  preferencesQueryKeys: { preferences: () => ['preferences'] },
  usePreferences: () => ({
    data: {
      jean_mcp_enabled: mocks.jeanMcpEnabled,
      has_seen_jean_mcp_intro: false,
    },
  }),
  usePatchPreferences: () => ({ mutate: mocks.patchPreferencesMutate }),
}))

vi.mock('@/hooks/useInstalledBackends', () => ({
  useInstalledBackends: () => ({
    installedBackends: ['codex', 'cursor'],
    isLoading: false,
  }),
}))

vi.mock('@/services/mcp', () => ({
  invalidateAllMcpServers: vi.fn(),
}))

const originalOpenPreferencesPane = useUIStore.getState().openPreferencesPane

function renderDialog() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  act(() => {
    useUIStore.setState({ jeanMcpIntroOpen: true })
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <JeanMcpIntroDialog />
    </QueryClientProvider>
  )
}

describe('JeanMcpIntroDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.jeanMcpEnabled = true
    act(() => {
      useUIStore.setState({ jeanMcpIntroOpen: false })
      useUIStore.setState({ openPreferencesPane: mocks.openPreferencesPane })
    })
    vi.mocked(invoke).mockResolvedValue([
      {
        backend: 'codex',
        status: 'installed',
        path: '/tmp/codex.toml',
        backupPath: null,
        serverName: 'jean',
        mode: 'prod',
        message: 'ok',
      },
      {
        backend: 'cursor',
        status: 'installed',
        path: '/tmp/mcp.json',
        backupPath: null,
        serverName: 'jean',
        mode: 'prod',
        message: 'ok',
      },
    ])
  })

  afterEach(() => {
    act(() => {
      useUIStore.setState({
        jeanMcpIntroOpen: false,
        openPreferencesPane: originalOpenPreferencesPane,
      })
    })
  })

  it('shows the enabled Jean MCP state', () => {
    renderDialog()

    expect(
      screen.getByRole('dialog', { name: /new: jean mcp server/i })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('switch', { name: /enable jean mcp/i })
    ).toBeChecked()
  })

  it('installs automatically and marks the intro as seen', async () => {
    const user = userEvent.setup()
    renderDialog()

    await user.click(screen.getByRole('button', { name: /add automatically/i }))

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('install_jean_mcp_config', {
        backends: ['codex', 'cursor'],
        mode: 'current',
      })
    })
    expect(mocks.patchPreferencesMutate).toHaveBeenCalledWith({
      jean_mcp_enabled: true,
      has_seen_jean_mcp_intro: true,
    })
  })

  it('can disable Jean MCP with the switch', async () => {
    const user = userEvent.setup()
    renderDialog()

    await user.click(screen.getByRole('switch', { name: /enable jean mcp/i }))

    expect(mocks.patchPreferencesMutate).toHaveBeenCalledWith({
      jean_mcp_enabled: false,
    })
  })

  it('marks as seen and opens MCP settings for manual setup', async () => {
    const user = userEvent.setup()
    renderDialog()

    await user.click(
      screen.getByRole('button', { name: /i’ll set it up manually/i })
    )

    expect(mocks.patchPreferencesMutate).toHaveBeenCalledWith({
      has_seen_jean_mcp_intro: true,
    })
    expect(mocks.openPreferencesPane).toHaveBeenCalledWith('mcp-servers')
  })
})
