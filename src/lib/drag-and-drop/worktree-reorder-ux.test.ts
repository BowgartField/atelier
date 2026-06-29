import { describe, expect, it } from 'vitest'
import {
  applyWorktreeDropSnapshot,
  getClosestVerticalEdge,
  getWorktreeDropTargetForScope,
} from './worktree-reorder-ux'
import {
  DRAG_SCOPE_CANVAS_WORKTREE_LIST,
  DRAG_SCOPE_WORKTREE_LIST,
} from './types'

describe('worktree reorder UX helpers', () => {
  it('detects top and bottom vertical edges from pointer position', () => {
    const rect = { top: 100, height: 40 }

    expect(getClosestVerticalEdge({ clientY: 119, rect })).toBe('top')
    expect(getClosestVerticalEdge({ clientY: 120, rect })).toBe('bottom')
  })

  it('keeps drag state referentially stable for unchanged target snapshots', () => {
    const state = {
      draggingId: 'source',
      targetId: 'target',
      closestEdge: 'top' as const,
    }

    expect(
      applyWorktreeDropSnapshot(state, {
        targetId: 'target',
        closestEdge: 'top',
      })
    ).toBe(state)
  })

  it('updates only target placement while preserving the dragging id', () => {
    expect(
      applyWorktreeDropSnapshot(
        {
          draggingId: 'source',
          targetId: null,
          closestEdge: null,
        },
        {
          targetId: 'target',
          closestEdge: 'bottom',
        }
      )
    ).toEqual({
      draggingId: 'source',
      targetId: 'target',
      closestEdge: 'bottom',
    })
  })

  it('selects worktree drop targets by scope', () => {
    const dropTargets = [
      {
        data: {
          type: 'worktree-section',
          projectId: 'project',
          worktreeId: 'sidebar-target',
          scope: DRAG_SCOPE_WORKTREE_LIST,
        },
      },
      {
        data: {
          type: 'worktree-section',
          projectId: 'project',
          worktreeId: 'canvas-target',
          scope: DRAG_SCOPE_CANVAS_WORKTREE_LIST,
        },
      },
    ]

    expect(
      getWorktreeDropTargetForScope(
        dropTargets,
        DRAG_SCOPE_CANVAS_WORKTREE_LIST
      )?.data.worktreeId
    ).toBe('canvas-target')
  })
})
