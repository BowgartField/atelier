import {
  codexModelOptions,
  modelOptions,
  type ClaudeModel,
} from '@/types/preferences'
import type { EffortLevel, ThinkingLevel } from '@/types/chat'

export const MODEL_OPTIONS: { value: ClaudeModel; label: string }[] =
  modelOptions.map(option => ({
    value: option.value,
    label: option.label.replace(/^Claude\s+/, ''),
  }))

export const CODEX_MODEL_OPTIONS = codexModelOptions as {
  value: string
  label: string
}[]

export const OPENCODE_MODEL_OPTIONS: { value: string; label: string }[] = [
  { value: 'opencode/gpt-5.3-codex', label: 'GPT-5.3 Codex (OpenCode)' },
]

export const COMMANDCODE_MODEL_OPTIONS: { value: string; label: string }[] = [
  { value: 'commandcode/default', label: 'CLI default (no --model)' },
]

export const CURSOR_MODEL_OPTIONS: { value: string; label: string }[] = [
  { value: 'cursor/composer-2.5-fast', label: 'Composer 2.5 Fast' },
  { value: 'cursor/composer-2.5', label: 'Composer 2.5' },
  { value: 'cursor/composer-2-fast', label: 'Composer 2 Fast' },
  { value: 'cursor/composer-2', label: 'Composer 2' },
  { value: 'cursor/gpt-5.5-high-fast', label: 'GPT-5.5 High Fast' },
  {
    value: 'cursor/claude-opus-4-7-thinking-high',
    label: 'Opus 4.7 1M High Thinking',
  },
  { value: 'cursor/auto', label: 'Auto' },
  { value: 'cursor/gemini-3.1-pro', label: 'Gemini 3.1 Pro' },
  { value: 'cursor/grok-4.3', label: 'Grok 4.3 1M' },
]

export const THINKING_LEVEL_OPTIONS: {
  value: ThinkingLevel
  label: string
  tokens: string
}[] = [
  { value: 'off', label: 'Off', tokens: 'Disabled' },
  { value: 'think', label: 'Think', tokens: '4K' },
  { value: 'megathink', label: 'Megathink', tokens: '10K' },
  { value: 'ultrathink', label: 'Ultrathink', tokens: '32K' },
]

export const EFFORT_LEVEL_OPTIONS: {
  value: EffortLevel
  label: string
  description: string
}[] = [
  { value: 'low', label: 'Low', description: 'Minimal' },
  { value: 'medium', label: 'Medium', description: 'Moderate' },
  { value: 'high', label: 'High', description: 'Deep' },
  { value: 'xhigh', label: 'xHigh', description: 'Extra high' },
  { value: 'max', label: 'Max', description: 'No limits' },
]

export const CODEX_EFFORT_LEVEL_OPTIONS = EFFORT_LEVEL_OPTIONS.filter(
  option => option.value !== 'max'
)
