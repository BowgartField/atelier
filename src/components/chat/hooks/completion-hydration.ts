import type { ContentBlock, ToolCall } from '@/types/chat'
import { splitTextAroundPlan } from '../tool-call-utils'

export function hasMeaningfulAssistantPayload(
  content: string,
  contentBlocks: ContentBlock[] = [],
  toolCalls: ToolCall[] = []
): boolean {
  if (content.trim().length > 0) return true
  if (toolCalls.length > 0) return true

  return contentBlocks.some(block => {
    switch (block.type) {
      case 'text':
        return block.text.trim().length > 0
      case 'thinking':
        return block.thinking.trim().length > 0
      case 'tool_use':
        return block.tool_call_id.trim().length > 0
    }
  })
}

const PLAN_TOOL_NAMES = new Set(['CodexPlan', 'ExitPlanMode'])

export function shouldHydrateCompletedSessionFromBackend(
  content: string,
  contentBlocks: ContentBlock[] = [],
  toolCalls: ToolCall[] = []
): boolean {
  // Always hydrate after a plan tool: chat:done adds an optimistic assistant
  // message with a frontend-generated id, which can race past useSendMessage
  // onSuccess and leave the cache holding an id that doesn't match the
  // backend's NDJSON id. mark_plan_approved keys on that id, so without
  // hydration the approval never sticks and the plan dialog re-shows.
  const hasPlanTool = toolCalls.some(tc => PLAN_TOOL_NAMES.has(tc.name))
  if (hasPlanTool) return true

  // No plan tool emitted, but text contains a plan section → hydrate so the
  // backend can re-parse and persist a structured plan.
  if (splitTextAroundPlan(content).plan) {
    return true
  }

  return !hasMeaningfulAssistantPayload(content, contentBlocks, toolCalls)
}
