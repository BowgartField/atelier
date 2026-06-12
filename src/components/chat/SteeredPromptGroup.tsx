import { memo } from 'react'

/**
 * Right-aligned card grouping one or more user prompts that were steered
 * into a running turn (Codex `turn/steer`). Consecutive steered prompts
 * render as connected rows in a single bubble so they read as one batch.
 */
export const SteeredPromptGroup = memo(function SteeredPromptGroup({
  texts,
}: {
  texts: string[]
}) {
  if (texts.length === 0) return null
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] sm:max-w-[70%] min-w-0 overflow-hidden rounded-lg border border-border bg-muted/20 divide-y divide-border/60">
        {texts.map((text, i) => (
          <div
            key={i}
            className="px-3 py-2 text-foreground break-words whitespace-pre-wrap"
          >
            {text}
          </div>
        ))}
      </div>
    </div>
  )
})
