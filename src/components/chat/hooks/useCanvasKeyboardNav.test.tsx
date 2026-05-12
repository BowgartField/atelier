import { fireEvent, render, cleanup } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useProjectsStore } from '@/store/projects-store'
import { useUIStore } from '@/store/ui-store'
import { useCanvasKeyboardNav } from './useCanvasKeyboardNav'

interface HarnessProps {
  cards?: string[]
  selectedIndex?: number | null
  enabled?: boolean
  onSelectedIndexChange?: (index: number | null) => void
  onSelect?: (index: number) => void
  onNavigateLeft?: () => void
  onNavigateRight?: () => void
  renderInput?: boolean
}

function Harness({
  cards = [],
  selectedIndex = null,
  enabled = true,
  onSelectedIndexChange = vi.fn(),
  onSelect = vi.fn(),
  onNavigateLeft,
  onNavigateRight,
  renderInput = false,
}: HarnessProps) {
  useCanvasKeyboardNav({
    cards,
    selectedIndex,
    onSelectedIndexChange,
    onSelect,
    onNavigateLeft,
    onNavigateRight,
    enabled,
  })

  return renderInput ? <input aria-label="Search" /> : null
}

describe('useCanvasKeyboardNav', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(1000)
    useUIStore.setState({
      magicModalOpen: false,
      planDialogOpen: false,
      commandPaletteOpen: false,
      preferencesOpen: false,
      releaseNotesModalOpen: false,
      sessionChatModalOpen: false,
    })
    useProjectsStore.setState({
      projectSettingsDialogOpen: false,
    })
  })

  afterEach(() => {
    cleanup()
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('calls left and right navigation callbacks even when there are no cards', () => {
    const onNavigateLeft = vi.fn()
    const onNavigateRight = vi.fn()

    render(
      <Harness
        onNavigateLeft={onNavigateLeft}
        onNavigateRight={onNavigateRight}
      />
    )

    fireEvent.keyDown(window, { key: 'ArrowLeft' })
    vi.advanceTimersByTime(100)
    fireEvent.keyDown(window, { key: 'ArrowRight' })

    expect(onNavigateLeft).toHaveBeenCalledTimes(1)
    expect(onNavigateRight).toHaveBeenCalledTimes(1)
  })

  it('keeps up and down navigation for card selection', () => {
    const onSelectedIndexChange = vi.fn()

    render(
      <Harness
        cards={['one', 'two']}
        selectedIndex={null}
        onSelectedIndexChange={onSelectedIndexChange}
      />
    )

    fireEvent.keyDown(window, { key: 'ArrowDown' })

    expect(onSelectedIndexChange).toHaveBeenCalledWith(0)
  })

  it('does not call tab navigation callbacks when keyboard nav is disabled', () => {
    const onNavigateRight = vi.fn()

    render(<Harness enabled={false} onNavigateRight={onNavigateRight} />)

    fireEvent.keyDown(window, { key: 'ArrowRight' })

    expect(onNavigateRight).not.toHaveBeenCalled()
  })

  it('does not call tab navigation callbacks while an input is focused', () => {
    const onNavigateRight = vi.fn()
    const { getByLabelText } = render(
      <Harness renderInput onNavigateRight={onNavigateRight} />
    )
    getByLabelText('Search').focus()

    fireEvent.keyDown(window, { key: 'ArrowRight' })

    expect(onNavigateRight).not.toHaveBeenCalled()
  })

  it('does not call tab navigation callbacks while a session modal is open', () => {
    const onNavigateRight = vi.fn()
    useUIStore.setState({ sessionChatModalOpen: true })

    render(<Harness onNavigateRight={onNavigateRight} />)

    fireEvent.keyDown(window, { key: 'ArrowRight' })

    expect(onNavigateRight).not.toHaveBeenCalled()
  })
})
