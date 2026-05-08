import { Button } from '@/components/ui/button'
import type { CodexDynamicToolCallRequest } from '@/types/chat'

interface CodexDynamicToolCallRequestProps {
  request: CodexDynamicToolCallRequest
  onRespondUnsupported: () => void
}

export function CodexDynamicToolCallRequest({
  request,
  onRespondUnsupported,
}: CodexDynamicToolCallRequestProps) {
  const toolName = request.namespace
    ? `${request.namespace}/${request.tool}`
    : request.tool

  return (
    <div className="my-3 rounded border border-muted bg-muted/30 p-4 font-mono text-sm">
      <div className="mb-2 font-semibold">
        Unsupported Codex dynamic tool call
      </div>
      <div className="mb-2 text-xs text-muted-foreground">Tool: {toolName}</div>
      <pre className="overflow-x-auto rounded bg-background/60 p-2 text-[11px] whitespace-pre-wrap break-words">
        {JSON.stringify(request.arguments, null, 2)}
      </pre>
      <div className="mt-4 flex gap-2">
        <Button size="sm" onClick={onRespondUnsupported}>
          Respond unsupported
        </Button>
      </div>
    </div>
  )
}
