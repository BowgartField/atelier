import type { Edge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { extractClosestEdge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { isWorktreeDropTargetData, type WorktreeDropTargetData } from './types'

export interface WorktreeReorderDragState {
  draggingId: string | null
  targetId: string | null
  closestEdge: Edge | null
}

export interface WorktreeDropSnapshot {
  targetId: string | null
  closestEdge: Edge | null
}

export const emptyWorktreeDropSnapshot: WorktreeDropSnapshot = {
  targetId: null,
  closestEdge: null,
}

export function getClosestVerticalEdge({
  clientY,
  rect,
}: {
  clientY: number
  rect: Pick<DOMRect, 'top' | 'height'>
}): Edge {
  return clientY < rect.top + rect.height / 2 ? 'top' : 'bottom'
}

export function getWorktreeDropTargetForScope(
  dropTargets: { data: Record<string | symbol, unknown> }[],
  scope: WorktreeDropTargetData['scope']
) {
  return dropTargets.find(
    dropTarget =>
      isWorktreeDropTargetData(dropTarget.data) &&
      dropTarget.data.scope === scope
  )
}

export function getSnapshotFromWorktreeDropTarget(
  dropTarget: { data: Record<string | symbol, unknown> } | undefined
): WorktreeDropSnapshot {
  if (!dropTarget || !isWorktreeDropTargetData(dropTarget.data)) {
    return emptyWorktreeDropSnapshot
  }

  return {
    targetId:
      typeof dropTarget.data.worktreeId === 'string'
        ? dropTarget.data.worktreeId
        : null,
    closestEdge: extractClosestEdge(dropTarget.data),
  }
}

export function getWorktreeElementFromEventTarget({
  eventTarget,
  scope,
}: {
  eventTarget: EventTarget | null
  scope: WorktreeDropTargetData['scope']
}): HTMLElement | null {
  return (
    (eventTarget as HTMLElement | null)?.closest(
      `[data-pdnd-worktree-scope="${scope}"]`
    ) ?? null
  )
}

export function getWorktreeElementFromPoint({
  clientX,
  clientY,
  scope,
}: {
  clientX: number
  clientY: number
  scope: WorktreeDropTargetData['scope']
}): HTMLElement | null {
  const element = document.elementFromPoint(
    clientX,
    clientY
  ) as HTMLElement | null
  return getWorktreeElementFromEventTarget({ eventTarget: element, scope })
}

export function getSnapshotFromWorktreeElement({
  element,
  draggingId,
  clientY,
}: {
  element: HTMLElement | null
  draggingId: string
  clientY: number
}): WorktreeDropSnapshot {
  const targetId = element?.dataset.pdndWorktreeId ?? null
  if (!element || !targetId || targetId === draggingId) {
    return emptyWorktreeDropSnapshot
  }

  return {
    targetId,
    closestEdge: getClosestVerticalEdge({
      clientY,
      rect: element.getBoundingClientRect(),
    }),
  }
}

export function applyWorktreeDropSnapshot(
  state: WorktreeReorderDragState,
  snapshot: WorktreeDropSnapshot
): WorktreeReorderDragState {
  if (
    state.targetId === snapshot.targetId &&
    state.closestEdge === snapshot.closestEdge
  ) {
    return state
  }

  return {
    ...state,
    targetId: snapshot.targetId,
    closestEdge: snapshot.closestEdge,
  }
}
