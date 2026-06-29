import { describe, expect, it } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { DropIndicator } from './DropIndicator'

describe('DropIndicator', () => {
  it('renders nothing when no edge is provided', () => {
    const { container } = render(<DropIndicator edge={null} />)

    expect(container).toBeEmptyDOMElement()
  })

  it('positions the indicator at the top edge', () => {
    render(<DropIndicator edge="top" />)

    expect(screen.getByTestId('drop-indicator')).toHaveClass(
      'top-0',
      '-translate-y-1/2'
    )
  })

  it('positions the indicator at the bottom edge', () => {
    render(<DropIndicator edge="bottom" />)

    expect(screen.getByTestId('drop-indicator')).toHaveClass(
      'bottom-0',
      'translate-y-1/2'
    )
  })

  it('accepts custom inset and wrapper classes', () => {
    render(
      <DropIndicator
        edge="top"
        className="custom-wrapper"
        insetClassName="left-5 right-0"
      />
    )

    expect(screen.getByTestId('drop-indicator')).toHaveClass(
      'custom-wrapper',
      'left-5',
      'right-0'
    )
  })
})
