import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { waitFor } from '@testing-library/react'
import { render } from '@/test/test-utils'
import { BrowserTabContent } from './BrowserTabContent'

const browserBackendMock = vi.hoisted(() => ({
  create: vi.fn(),
  setBounds: vi.fn(),
  setVisible: vi.fn(),
  hasActive: vi.fn(),
  close: vi.fn(),
}))

vi.mock('@/hooks/useBrowserPane', () => ({
  browserBackend: browserBackendMock,
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onScaleChanged: vi.fn().mockResolvedValue(vi.fn()),
  }),
}))

class ResizeObserverMock {
  observe = vi.fn()
  disconnect = vi.fn()
}

describe('BrowserTabContent', () => {
  beforeEach(() => {
    vi.stubGlobal('ResizeObserver', ResizeObserverMock)
    vi.stubGlobal(
      'requestAnimationFrame',
      vi.fn(() => 1)
    )
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
    browserBackendMock.create.mockResolvedValue(undefined)
    browserBackendMock.setBounds.mockResolvedValue(undefined)
    browserBackendMock.setVisible.mockResolvedValue(undefined)
    browserBackendMock.hasActive.mockResolvedValue(false)
    browserBackendMock.close.mockResolvedValue(undefined)
  })

  afterEach(() => {
    vi.unstubAllGlobals()
    vi.clearAllMocks()
  })

  it('does not park a tab on unmount after the backend tab is already closed', async () => {
    const { unmount } = render(
      <BrowserTabContent tabId="tab-1" isActive={false} />
    )

    unmount()

    await waitFor(() => {
      expect(browserBackendMock.hasActive).toHaveBeenCalledWith('tab-1')
    })
    expect(browserBackendMock.setBounds).not.toHaveBeenCalled()
    expect(browserBackendMock.setVisible).not.toHaveBeenCalled()
  })
})
