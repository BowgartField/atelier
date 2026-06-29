import { describe, expect, it, vi, beforeEach } from 'vitest'
import { useEffect, useRef } from 'react'
import { render, waitFor } from '@/test/test-utils'
import { useTerminal } from './useTerminal'

const terminalInstanceMocks = vi.hoisted(() => ({
  getOrCreateTerminal: vi.fn(),
  attachToContainer: vi.fn().mockResolvedValue(undefined),
  detachFromContainer: vi.fn(),
  fitTerminal: vi.fn(),
  focusTerminal: vi.fn(),
}))

vi.mock('@/lib/terminal-instances', () => ({
  getOrCreateTerminal: terminalInstanceMocks.getOrCreateTerminal,
  attachToContainer: terminalInstanceMocks.attachToContainer,
  detachFromContainer: terminalInstanceMocks.detachFromContainer,
  fitTerminal: terminalInstanceMocks.fitTerminal,
  focusTerminal: terminalInstanceMocks.focusTerminal,
}))

function TerminalHarness({ terminalId }: { terminalId: string }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const { initTerminal } = useTerminal({
    terminalId,
    worktreeId: 'worktree-1',
    worktreePath: '/tmp/worktree-1',
  })

  useEffect(() => {
    if (containerRef.current) {
      void initTerminal(containerRef.current)
    }
  }, [initTerminal])

  return <div ref={containerRef} data-testid="terminal-container" />
}

describe('useTerminal', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    terminalInstanceMocks.attachToContainer.mockResolvedValue(undefined)
  })

  it('detaches the previous terminal when the host opens a different terminal id', async () => {
    const { rerender } = render(<TerminalHarness terminalId="terminal-1" />)

    await waitFor(() =>
      expect(terminalInstanceMocks.attachToContainer).toHaveBeenCalledWith(
        'terminal-1',
        expect.any(HTMLDivElement)
      )
    )

    rerender(<TerminalHarness terminalId="terminal-2" />)

    await waitFor(() =>
      expect(terminalInstanceMocks.attachToContainer).toHaveBeenCalledWith(
        'terminal-2',
        expect.any(HTMLDivElement)
      )
    )
    expect(terminalInstanceMocks.detachFromContainer).toHaveBeenCalledWith(
      'terminal-1'
    )
  })

  it('does not reattach the same terminal id while already attached', async () => {
    const { rerender } = render(<TerminalHarness terminalId="terminal-1" />)

    await waitFor(() =>
      expect(terminalInstanceMocks.attachToContainer).toHaveBeenCalledTimes(1)
    )

    rerender(<TerminalHarness terminalId="terminal-1" />)

    expect(terminalInstanceMocks.attachToContainer).toHaveBeenCalledTimes(1)
    expect(terminalInstanceMocks.detachFromContainer).not.toHaveBeenCalled()
  })
})
