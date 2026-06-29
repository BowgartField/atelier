import { describe, expect, it } from 'vitest'
import { reorderWithClosestEdge } from './reorder'

describe('reorderWithClosestEdge', () => {
  it('moves an item before the target for top edge drops', () => {
    expect(
      reorderWithClosestEdge({
        items: ['a', 'b', 'c'],
        startIndex: 0,
        indexOfTarget: 2,
        closestEdgeOfTarget: 'top',
      })
    ).toEqual(['b', 'a', 'c'])
  })

  it('moves an item after the target for bottom edge drops', () => {
    expect(
      reorderWithClosestEdge({
        items: ['a', 'b', 'c'],
        startIndex: 0,
        indexOfTarget: 2,
        closestEdgeOfTarget: 'bottom',
      })
    ).toEqual(['b', 'c', 'a'])
  })

  it('handles dragging upward before the target', () => {
    expect(
      reorderWithClosestEdge({
        items: ['a', 'b', 'c'],
        startIndex: 2,
        indexOfTarget: 0,
        closestEdgeOfTarget: 'top',
      })
    ).toEqual(['c', 'a', 'b'])
  })
})
