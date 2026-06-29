import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { FullScreenTerminalSurface } from './FullScreenTerminalSurface'

vi.mock('./TerminalView', () => ({
  SingleTerminalView: ({
    terminalId,
    worktreeId,
    worktreePath,
    isActive,
  }: {
    terminalId: string
    worktreeId: string
    worktreePath: string
    isActive?: boolean
  }) => (
    <div data-testid="terminal-view">
      {terminalId}:{worktreeId}:{worktreePath}:{String(isActive)}
    </div>
  ),
}))

describe('FullScreenTerminalSurface', () => {
  it('renders through TerminalView so the selected emulator is respected', () => {
    const { container } = render(
      <FullScreenTerminalSurface
        worktreeId="worktree-1"
        worktreePath="/tmp/worktree-1"
        terminalId="terminal-1"
      />
    )

    expect(screen.getByTestId('terminal-view')).toHaveTextContent(
      'terminal-1:worktree-1:/tmp/worktree-1:true'
    )
    expect(container.firstElementChild).toHaveAttribute(
      'data-terminal-root',
      'true'
    )
    expect(container.firstElementChild).toHaveAttribute(
      'data-terminal-surface',
      'session'
    )
  })

  it('passes inactive state down so hidden terminal sessions do not start', () => {
    render(
      <FullScreenTerminalSurface
        worktreeId="worktree-1"
        worktreePath="/tmp/worktree-1"
        terminalId="terminal-1"
        isActive={false}
      />
    )

    expect(screen.getByTestId('terminal-view')).toHaveTextContent(
      'terminal-1:worktree-1:/tmp/worktree-1:false'
    )
  })

  it('does not show a header by default for full-screen terminals', () => {
    render(
      <FullScreenTerminalSurface
        worktreeId="worktree-1"
        worktreePath="/tmp/worktree-1"
        terminalId="terminal-1"
      />
    )

    expect(screen.queryByRole('button', { name: /chat/i })).toBeNull()
  })
})
