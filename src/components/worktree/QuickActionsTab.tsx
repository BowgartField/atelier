import { useState } from 'react'
import { GitBranch, Loader2, Plus, Settings } from 'lucide-react'
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { useUIStore } from '@/store/ui-store'
import { useProjectsStore } from '@/store/projects-store'
import { normalizeRunScripts, type JeanConfig } from '@/services/projects'

export interface QuickActionsTabProps {
  hasBaseSession: boolean
  onCreateWorktree: (customName?: string) => void
  onBaseSession: () => void
  isCreating: boolean
  projectId: string | null
  jeanConfig: JeanConfig | null | undefined
}

const INVALID_BRANCH_CHAR = /[\s:?*~^[\\]/

function isInvalidBranchName(trimmed: string): boolean {
  if (trimmed.length === 0) return false
  return (
    INVALID_BRANCH_CHAR.test(trimmed) ||
    trimmed.startsWith('/') ||
    trimmed.endsWith('/') ||
    trimmed.startsWith('.') ||
    trimmed.endsWith('.') ||
    trimmed.includes('..') ||
    trimmed.endsWith('.lock')
  )
}

export function QuickActionsTab({
  hasBaseSession,
  onCreateWorktree,
  onBaseSession,
  isCreating,
  projectId,
  jeanConfig,
}: QuickActionsTabProps) {
  const [customBranchName, setCustomBranchName] = useState('')
  const setupScript = jeanConfig?.scripts.setup
  const runScripts = normalizeRunScripts(jeanConfig?.scripts.run)

  const trimmedBranchName = customBranchName.trim()
  const isInvalid = isInvalidBranchName(trimmedBranchName)

  const handleRunClick = () => {
    if (!projectId) return
    if (runScripts.length === 0) {
      useUIStore.getState().setNewWorktreeModalOpen(false)
      useProjectsStore.getState().openProjectSettings(projectId, 'jean-json')
    }
  }

  const handleCreateClick = () => {
    if (isInvalid) return
    onCreateWorktree(trimmedBranchName || undefined)
    setCustomBranchName('')
  }

  return (
    <div className="flex flex-col items-center justify-center flex-1 p-4 sm:p-10">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-4 sm:gap-6 w-full max-w-xl">
        {/* Base Session button */}
        <button
          onClick={onBaseSession}
          disabled={isCreating}
          className={cn(
            'relative flex flex-col items-center justify-center gap-4 p-4 sm:p-8 sm:h-full rounded-xl text-sm transition-colors',
            'hover:bg-accent focus:outline-none focus:ring-2 focus:ring-ring',
            'border border-border bg-card'
          )}
        >
          <GitBranch className="h-10 w-10 shrink-0 text-muted-foreground" />
          <div className="flex flex-col items-center gap-1.5">
            <span className="font-medium text-base">
              {hasBaseSession ? 'Switch to Base Session' : 'New Base Session'}
            </span>
            <span className="text-xs text-muted-foreground text-center">
              Work directly on the project folder
            </span>
          </div>
          <kbd className="hidden sm:block absolute top-3 right-3 text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
            M
          </kbd>
        </button>

        {/* New Worktree button */}
        <div
          className={cn(
            'relative flex flex-col items-center justify-center gap-3 sm:gap-4 sm:aspect-square p-4 sm:p-8 rounded-xl text-sm transition-colors',
            'border border-border bg-card'
          )}
        >
          {isCreating ? (
            <Loader2 className="h-10 w-10 shrink-0 text-muted-foreground animate-spin" />
          ) : (
            <Plus className="h-10 w-10 shrink-0 text-muted-foreground" />
          )}
          <div className="flex flex-col items-center gap-1.5">
            <span className="font-medium text-base">New Worktree</span>
            <span className="text-xs text-muted-foreground text-center">
              Create an isolated branch for your task
            </span>
            {setupScript && (
              <span className="text-xs text-muted-foreground/70 font-mono truncate max-w-[200px]">
                Setup: {setupScript}
              </span>
            )}
          </div>
          <input
            type="text"
            value={customBranchName}
            onChange={e => setCustomBranchName(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && !isCreating && !isInvalid)
                handleCreateClick()
            }}
            placeholder="Branch name (optional)"
            disabled={isCreating}
            aria-invalid={isInvalid}
            className={cn(
              'mt-1 w-full max-w-[180px] px-2 py-1 text-xs text-center rounded border bg-background focus:outline-none focus:ring-1 focus:ring-ring disabled:opacity-50',
              isInvalid ? 'border-destructive' : 'border-border'
            )}
          />
          {isInvalid && (
            <span className="text-xs text-destructive">
              Invalid branch name
            </span>
          )}
          <button
            onClick={handleCreateClick}
            disabled={isCreating || isInvalid}
            className={cn(
              'px-3 py-1 rounded text-xs transition-colors',
              'bg-primary text-primary-foreground hover:bg-primary/90',
              'focus:outline-none focus:ring-2 focus:ring-ring',
              'disabled:opacity-50'
            )}
          >
            Create
          </button>
          <kbd className="hidden sm:block absolute top-3 right-3 text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
            N
          </kbd>
        </div>
      </div>

      {/* Configure jean.json - only show when not configured */}
      {runScripts.length === 0 && projectId && (
        <div className="flex items-center gap-1 mt-6">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                onClick={handleRunClick}
                className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs text-muted-foreground/40 hover:text-foreground hover:bg-accent cursor-pointer transition-colors"
              >
                <Settings className="h-4 w-4" />
                <span>Configure jean.json</span>
              </button>
            </TooltipTrigger>
            <TooltipContent side="bottom">Configure jean.json</TooltipContent>
          </Tooltip>
        </div>
      )}
    </div>
  )
}
