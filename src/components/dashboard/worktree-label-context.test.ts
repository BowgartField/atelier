import { describe, expect, it } from 'vitest'
import {
  shouldDisableWorktreeTextSelection,
  shouldShowWorktreeLabelContextMenu,
} from './worktree-label-context'

describe('shouldShowWorktreeLabelContextMenu', () => {
  it('shows labels menu on mobile and native desktop', () => {
    expect(
      shouldShowWorktreeLabelContextMenu({ isMobile: true, isNative: false })
    ).toBe(true)
    expect(
      shouldShowWorktreeLabelContextMenu({ isMobile: false, isNative: true })
    ).toBe(true)
    expect(
      shouldShowWorktreeLabelContextMenu({ isMobile: false, isNative: false })
    ).toBe(false)
  })
})

describe('shouldDisableWorktreeTextSelection', () => {
  it('disables selection on mobile long-press targets only', () => {
    expect(shouldDisableWorktreeTextSelection({ isMobile: true })).toBe(true)
    expect(shouldDisableWorktreeTextSelection({ isMobile: false })).toBe(false)
  })
})
