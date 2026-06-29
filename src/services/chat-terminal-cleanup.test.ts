import { beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanupSessionTerminalForRemovedSession } from './chat'
import { useTerminalStore } from '@/store/terminal-store'
import { useUIStore } from '@/store/ui-store'

const { mockDisposeTerminal, mockInvoke } = vi.hoisted(() => ({
  mockDisposeTerminal: vi.fn(),
  mockInvoke: vi.fn().mockResolvedValue(true),
}))

vi.mock('@/lib/terminal-instances', () => ({
  disposeTerminal: mockDisposeTerminal,
}))

vi.mock('@/lib/transport', () => ({
  invoke: mockInvoke,
  listen: vi.fn().mockResolvedValue(() => undefined),
}))

vi.mock('@/services/projects', () => ({
  isTauri: () => true,
  projectsQueryKeys: { all: ['projects'] },
}))

describe('cleanupSessionTerminalForRemovedSession', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    useUIStore.setState({
      sessionPrimarySurface: {},
      sessionTerminalIds: {},
    })
    useTerminalStore.setState({
      terminals: {},
      activeTerminalIds: {},
      runningTerminals: new Set(),
      failedTerminals: new Set(),
      terminalVisible: false,
      terminalPanelOpen: {},
      modalTerminalOpen: {},
    })
  })

  it('stops, disposes, and clears a session-owned terminal', () => {
    useUIStore.setState({
      sessionPrimarySurface: { 'session-1': 'terminal' },
      sessionTerminalIds: { 'session-1': 'terminal-1' },
    })
    useTerminalStore.setState({
      terminals: {
        'worktree-1': [
          {
            id: 'terminal-1',
            worktreeId: 'worktree-1',
            command: 'codex',
            label: 'Codex',
            kind: 'session',
          },
        ],
      },
      runningTerminals: new Set(['terminal-1']),
    })

    const terminalId = cleanupSessionTerminalForRemovedSession(
      'worktree-1',
      'session-1'
    )

    expect(terminalId).toBe('terminal-1')
    expect(mockInvoke).toHaveBeenCalledWith('stop_terminal', {
      terminalId: 'terminal-1',
    })
    expect(mockDisposeTerminal).toHaveBeenCalledWith('terminal-1')
    expect(useUIStore.getState().sessionPrimarySurface).not.toHaveProperty(
      'session-1'
    )
    expect(useUIStore.getState().sessionTerminalIds).not.toHaveProperty(
      'session-1'
    )
    expect(useTerminalStore.getState().terminals['worktree-1']).toEqual([])
    expect(useTerminalStore.getState().runningTerminals.has('terminal-1')).toBe(
      false
    )
  })

  it('is a no-op for normal chat sessions', () => {
    useUIStore.setState({
      sessionPrimarySurface: { 'session-1': 'chat' },
      sessionTerminalIds: {},
    })

    const terminalId = cleanupSessionTerminalForRemovedSession(
      'worktree-1',
      'session-1'
    )

    expect(terminalId).toBeUndefined()
    expect(mockInvoke).not.toHaveBeenCalled()
    expect(mockDisposeTerminal).not.toHaveBeenCalled()
    expect(useUIStore.getState().sessionPrimarySurface).not.toHaveProperty(
      'session-1'
    )
  })
})
