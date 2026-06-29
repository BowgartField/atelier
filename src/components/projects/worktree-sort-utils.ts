import {
  isBaseSession,
  type Worktree,
  type WorktreeSortMode,
} from '@/types/projects'
import type { Session } from '@/types/chat'

export type { WorktreeSortMode } from '@/types/projects'

export function getSessionActivityTimestamp(session: Session): number {
  return session.last_message_at ?? session.updated_at ?? session.created_at
}

export function getWorktreeLastActivity(
  sessions: Session[],
  fallbackTimestamp: number
): number {
  return sessions.reduce(
    (latest, session) => Math.max(latest, getSessionActivityTimestamp(session)),
    fallbackTimestamp
  )
}

export function getWorktreeSortValue(
  worktree: Worktree,
  latestActivityAt: number,
  sortMode: WorktreeSortMode
): number {
  if (sortMode === 'manual') {
    return worktree.order
  }

  if (sortMode === 'created') {
    return worktree.created_at
  }

  return Math.max(latestActivityAt, worktree.created_at)
}

export function compareWorktreesForCanvasSort(
  a: Worktree,
  b: Worktree,
  latestActivityByWorktreeId: ReadonlyMap<string, number>,
  sortMode: WorktreeSortMode
): number {
  const aIsBase = isBaseSession(a)
  const bIsBase = isBaseSession(b)
  if (aIsBase && !bIsBase) return -1
  if (!aIsBase && bIsBase) return 1

  if (sortMode === 'manual') {
    const orderDiff = a.order - b.order
    if (orderDiff !== 0) return orderDiff

    const createdDiff = b.created_at - a.created_at
    if (createdDiff !== 0) return createdDiff

    return a.id.localeCompare(b.id)
  }

  const sortDiff =
    getWorktreeSortValue(
      b,
      latestActivityByWorktreeId.get(b.id) ?? b.created_at,
      sortMode
    ) -
    getWorktreeSortValue(
      a,
      latestActivityByWorktreeId.get(a.id) ?? a.created_at,
      sortMode
    )
  if (sortDiff !== 0) return sortDiff

  return b.created_at - a.created_at
}
