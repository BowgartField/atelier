import { reorder } from '@atlaskit/pragmatic-drag-and-drop/reorder'
import type { Edge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { getReorderDestinationIndex } from '@atlaskit/pragmatic-drag-and-drop-hitbox/util/get-reorder-destination-index'

export function reorderWithClosestEdge<T>({
  items,
  startIndex,
  indexOfTarget,
  closestEdgeOfTarget,
}: {
  items: T[]
  startIndex: number
  indexOfTarget: number
  closestEdgeOfTarget: Edge | null
}): T[] {
  const finishIndex = getReorderDestinationIndex({
    startIndex,
    indexOfTarget,
    closestEdgeOfTarget,
    axis: 'vertical',
  })

  return reorder({
    list: items,
    startIndex,
    finishIndex,
  })
}

export function getReorderIndexWithClosestEdge({
  startIndex,
  indexOfTarget,
  closestEdgeOfTarget,
}: {
  startIndex: number
  indexOfTarget: number
  closestEdgeOfTarget: Edge | null
}): number {
  return getReorderDestinationIndex({
    startIndex,
    indexOfTarget,
    closestEdgeOfTarget,
    axis: 'vertical',
  })
}
