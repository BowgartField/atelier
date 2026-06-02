import { describe, expect, it, vi } from 'vitest'

import { triggerLatestToastAction } from '@/components/ui/sonner'

describe('triggerLatestToastAction', () => {
  it('runs the newest toast action', () => {
    const olderAction = vi.fn()
    const newestAction = vi.fn()

    const handled = triggerLatestToastAction([
      {
        id: 'older',
        action: { label: 'Open', onClick: olderAction },
      },
      {
        id: 'newest',
        action: { label: 'Resolve Conflicts', onClick: newestAction },
      },
    ])

    expect(handled).toBe(true)
    expect(olderAction).not.toHaveBeenCalled()
    expect(newestAction).toHaveBeenCalledTimes(1)
  })

  it('ignores toasts without object actions', () => {
    const handled = triggerLatestToastAction([
      { id: 'plain' },
      { id: 'custom', action: <button type="button">Custom</button> },
    ])

    expect(handled).toBe(false)
  })
})
