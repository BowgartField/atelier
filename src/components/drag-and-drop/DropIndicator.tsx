import type { Edge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge'
import { cn } from '@/lib/utils'

interface DropIndicatorProps {
  edge: Edge | null
  className?: string
  insetClassName?: string
}

export function DropIndicator({
  edge,
  className,
  insetClassName = 'left-2 right-2',
}: DropIndicatorProps) {
  if (!edge) return null

  return (
    <div
      aria-hidden="true"
      data-testid="drop-indicator"
      className={cn(
        'pointer-events-none absolute z-30 h-2',
        insetClassName,
        edge === 'top' ? 'top-0 -translate-y-1/2' : 'bottom-0 translate-y-1/2',
        className
      )}
    >
      <div className="absolute left-0 right-0 top-1/2 h-0.5 -translate-y-1/2 rounded-full bg-primary shadow-[0_0_0_1px_hsl(var(--primary)/0.15),0_1px_4px_hsl(var(--primary)/0.35)]" />
      <div className="absolute left-0 top-1/2 size-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-primary shadow-[0_0_0_2px_hsl(var(--background))]" />
    </div>
  )
}
