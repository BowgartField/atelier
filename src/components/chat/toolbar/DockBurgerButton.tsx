import { useCallback, useEffect, useRef, useState } from 'react'
import {
  Archive,
  Command,
  FileText,
  Github,
  GitPullRequest,
  LayoutDashboard,
  Menu,
  Paperclip,
  Plug,
  Plus,
  ShieldAlert,
  Sparkles,
  Terminal,
} from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { cn } from '@/lib/utils'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { useIsMobile } from '@/hooks/use-mobile'
import { invoke } from '@/lib/transport'
import { useWsConnectionStatus } from '@/lib/transport'
import { isNativeApp } from '@/lib/environment'
import { openExternal, preOpenWindow } from '@/lib/platform'
import { copyToClipboard } from '@/lib/clipboard'
import { useUIStore } from '@/store/ui-store'
import { useChatStore } from '@/store/chat-store'
import { useProjectsStore } from '@/store/projects-store'
import { chatQueryKeys } from '@/services/chat'
import { usePreferences } from '@/services/preferences'
import { useWorktree, type GitHubRemote } from '@/services/projects'
import {
  useCodexCliAuth,
  useCodexCliStatus,
  useCodexUsage,
} from '@/services/codex-cli'
import type { WorktreeSessions } from '@/types/chat'
import { DEFAULT_KEYBINDINGS, formatShortcutDisplay } from '@/types/keybindings'
import { getResumeCommand } from '@/components/chat/session-card-utils'

interface DockBurgerButtonProps {
  /** Number of enabled MCP servers; shown as a badge next to the MCP item. */
  activeMcpCount?: number
  /** Attach-images handler — opens native file picker (see ChatWindow). */
  onAttach?: () => void
  /** Extra classes merged onto the trigger button (e.g. responsive visibility). */
  className?: string
}

export function DockBurgerButton({
  activeMcpCount = 0,
  onAttach,
  className,
}: DockBurgerButtonProps = {}) {
  const isMobile = useIsMobile()
  const queryClient = useQueryClient()
  const { data: preferences } = usePreferences()

  const selectedProjectId = useProjectsStore(state => state.selectedProjectId)
  const selectedWorktreeId = useProjectsStore(state => state.selectedWorktreeId)
  const activeWorktreeId = useChatStore(state => state.activeWorktreeId)
  const sessionChatModalOpen = useUIStore(state => state.sessionChatModalOpen)
  const sessionChatModalWorktreeId = useUIStore(
    state => state.sessionChatModalWorktreeId
  )
  const currentWorktreeId = sessionChatModalOpen
    ? (sessionChatModalWorktreeId ?? activeWorktreeId ?? selectedWorktreeId)
    : (activeWorktreeId ?? selectedWorktreeId)
  const { data: worktree } = useWorktree(isMobile ? currentWorktreeId : null)
  const activeSessionId = useChatStore(state =>
    currentWorktreeId ? state.activeSessionIds[currentWorktreeId] : undefined
  )
  const selectedBackend = useChatStore(state =>
    activeSessionId ? state.selectedBackends[activeSessionId] : undefined
  )

  const [menuOpen, setMenuOpen] = useState(false)
  const [resumeCommand, setResumeCommand] = useState<string | null>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  const activeBackend = (selectedBackend ??
    preferences?.default_backend ??
    'claude') as 'claude' | 'codex' | 'opencode' | 'cursor'

  const codexStatus = useCodexCliStatus()
  const codexAuth = useCodexCliAuth({
    enabled: !!codexStatus.data?.installed,
  })
  const codexUsage = useCodexUsage({
    enabled:
      !!codexStatus.data?.installed &&
      !!codexAuth.data?.authenticated &&
      menuOpen,
  })

  const codexAvailable =
    !!codexStatus.data?.installed && !!codexAuth.data?.authenticated
  const showCodexUsage = activeBackend === 'codex' && codexAvailable
  const sessionPct = codexUsage.data?.session?.usedPercent ?? null
  const weeklyPct = codexUsage.data?.weekly?.usedPercent ?? null
  const planText =
    codexUsage.data?.planType && codexUsage.data.planType.trim().length > 0
      ? codexUsage.data.planType
      : '--'
  const sessionText = sessionPct === null ? '--' : `${Math.round(sessionPct)}`
  const weeklyText = weeklyPct === null ? '--' : `${Math.round(weeklyPct)}`

  const getActiveResumeCommand = useCallback(() => {
    const { selectedWorktreeId: worktreeId } = useProjectsStore.getState()
    if (!worktreeId) return null
    const sessionId = useChatStore.getState().activeSessionIds[worktreeId]
    if (!sessionId) return null
    const cached =
      queryClient.getQueryData<WorktreeSessions>(
        chatQueryKeys.sessions(worktreeId)
      ) ??
      queryClient.getQueryData<WorktreeSessions>([
        ...chatQueryKeys.sessions(worktreeId),
        'with-counts',
      ])
    const session = cached?.sessions?.find(s => s.id === sessionId)
    return session ? getResumeCommand(session) : null
  }, [queryClient])

  const handleOpenChange = useCallback(
    (open: boolean) => {
      setMenuOpen(open)
      if (open) setResumeCommand(getActiveResumeCommand())
    },
    [getActiveResumeCommand]
  )

  const toggleMenu = useCallback(() => {
    setMenuOpen(prev => {
      const next = !prev
      if (next) setResumeCommand(getActiveResumeCommand())
      return next
    })
  }, [getActiveResumeCommand])

  // Global shortcut — only respond when this instance is the visible variant.
  // Both desktop + mobile burgers mount; CSS (`hidden`/`@xl:hidden`) hides one.
  // `offsetParent === null` is true for `display: none`, so the hidden variant skips.
  useEffect(() => {
    const handler = () => {
      if (!triggerRef.current || triggerRef.current.offsetParent === null)
        return
      toggleMenu()
    }
    window.addEventListener('toggle-quick-menu', handler)
    return () => window.removeEventListener('toggle-quick-menu', handler)
  }, [toggleMenu])

  const handleCopyResumeCommand = useCallback(() => {
    const commandToCopy = getActiveResumeCommand() ?? resumeCommand
    if (!commandToCopy) return
    void copyToClipboard(commandToCopy)
      .then(() => toast.success('Resume command copied'))
      .catch(() => toast.error('Failed to copy resume command'))
  }, [getActiveResumeCommand, resumeCommand])

  const handleOpenGitHub = useCallback(() => {
    const branch = worktree?.branch
    if (!branch) {
      if (isNativeApp()) {
        if (selectedProjectId) {
          invoke('open_project_on_github', { projectId: selectedProjectId })
        }
      } else {
        const targetPath = worktree?.path
        if (targetPath) {
          const win = preOpenWindow()
          invoke<string>('get_github_repo_url', { repoPath: targetPath })
            .then(url => openExternal(url, win))
            .catch(() => {
              win?.close()
              toast.error('Failed to open GitHub')
            })
        }
      }
      return
    }
    const targetPath = worktree?.path
    if (!targetPath) return
    const win = preOpenWindow()
    invoke<GitHubRemote[]>('get_github_remotes', { repoPath: targetPath })
      .then(remotes => {
        if (!remotes || remotes.length <= 1) {
          const url = remotes?.[0]?.url
          if (url) openExternal(`${url}/tree/${branch}`, win)
          else win?.close()
        } else {
          win?.close()
          useUIStore.getState().openRemotePicker(targetPath, remoteName => {
            const remote = remotes.find(r => r.name === remoteName)
            if (remote) openExternal(`${remote.url}/tree/${branch}`)
          })
        }
      })
      .catch(() => {
        win?.close()
        toast.error('Failed to fetch remotes')
      })
  }, [worktree?.branch, worktree?.path, selectedProjectId])

  const handleOpenPR = useCallback(() => {
    if (worktree?.pr_url) openExternal(worktree.pr_url)
  }, [worktree?.pr_url])

  const handleOpenSecurityAlert = useCallback(() => {
    const url = worktree?.security_alert_url ?? worktree?.advisory_url
    if (url) openExternal(url)
  }, [worktree?.security_alert_url, worktree?.advisory_url])

  const githubShortcut = formatShortcutDisplay(
    (preferences?.keybindings?.open_github_dashboard ??
      DEFAULT_KEYBINDINGS.open_github_dashboard) as string
  )
  const menuShortcut = formatShortcutDisplay(
    (preferences?.keybindings?.open_quick_menu ??
      DEFAULT_KEYBINDINGS.open_quick_menu) as string
  )

  const isWebAccess = !isNativeApp()
  const connected = useWsConnectionStatus()
  const showConnectionIndicator = isWebAccess

  return (
    <DropdownMenu open={menuOpen} onOpenChange={handleOpenChange}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <button
              ref={triggerRef}
              type="button"
              aria-label={`Menu (${menuShortcut})`}
              className={cn(
                'flex h-8 items-center gap-1 px-2 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/80 hover:text-foreground',
                className
              )}
            >
              <Menu className="h-3.5 w-3.5" />
            </button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>Menu ({menuShortcut})</TooltipContent>
      </Tooltip>
      <DropdownMenuContent
        side="top"
        align="start"
        className="min-w-[240px]"
        onEscapeKeyDown={e => e.stopPropagation()}
      >
        {onAttach && (
          <DropdownMenuItem onClick={onAttach}>
            <Paperclip className="mr-2 h-4 w-4" />
            Attachments
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          onClick={() =>
            useUIStore.getState().openPreferencesPane('mcp-servers')
          }
        >
          <Plug
            className={
              activeMcpCount > 0
                ? 'mr-2 h-4 w-4 text-emerald-600 dark:text-emerald-400'
                : 'mr-2 h-4 w-4'
            }
          />
          MCP Servers
          {activeMcpCount > 0 && (
            <DropdownMenuShortcut>{activeMcpCount}</DropdownMenuShortcut>
          )}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onClick={() =>
            useProjectsStore.getState().setAddProjectDialogOpen(true)
          }
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Project
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => useUIStore.getState().setGitHubDashboardOpen(true)}
        >
          <LayoutDashboard className="mr-2 h-4 w-4" />
          GitHub Dashboard
          <DropdownMenuShortcut>{githubShortcut}</DropdownMenuShortcut>
        </DropdownMenuItem>
        {resumeCommand && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={handleCopyResumeCommand}>
              <Terminal className="mr-2 h-4 w-4" />
              Copy Resume Command
            </DropdownMenuItem>
          </>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onClick={() => window.dispatchEvent(new CustomEvent('open-recap'))}
        >
          <Sparkles className="mr-2 h-4 w-4" />
          View Recap
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => window.dispatchEvent(new CustomEvent('open-plan'))}
        >
          <FileText className="mr-2 h-4 w-4" />
          View Plan
        </DropdownMenuItem>
        {isMobile && currentWorktreeId && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={handleOpenGitHub}>
              <Github className="mr-2 h-4 w-4" />
              GitHub
            </DropdownMenuItem>
            {worktree?.pr_url && (
              <DropdownMenuItem onClick={handleOpenPR}>
                <GitPullRequest className="mr-2 h-4 w-4" />
                PR #{worktree.pr_number}
              </DropdownMenuItem>
            )}
            {(worktree?.security_alert_url || worktree?.advisory_url) && (
              <DropdownMenuItem onClick={handleOpenSecurityAlert}>
                <ShieldAlert className="mr-2 h-4 w-4" />
                {worktree?.security_alert_number
                  ? `Alert #${worktree.security_alert_number}`
                  : worktree?.advisory_ghsa_id}
              </DropdownMenuItem>
            )}
          </>
        )}

        <DropdownMenuSeparator />
        <DropdownMenuItem
          onClick={() => useUIStore.getState().setCommandPaletteOpen(true)}
        >
          <Command className="mr-2 h-4 w-4" />
          Command Palette
          <DropdownMenuShortcut>⌘K</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() =>
            window.dispatchEvent(new CustomEvent('command:open-archived-modal'))
          }
        >
          <Archive className="mr-2 h-4 w-4" />
          Archives
        </DropdownMenuItem>

        {!isMobile && showCodexUsage && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel className="text-[11px] text-muted-foreground">
              Codex usage · Plan: {planText}
            </DropdownMenuLabel>
            <DropdownMenuItem
              onClick={() => useUIStore.getState().openPreferencesPane('usage')}
            >
              Session | Weekly
              <DropdownMenuShortcut>
                {sessionText}|{weeklyText}%
              </DropdownMenuShortcut>
            </DropdownMenuItem>
          </>
        )}

        {showConnectionIndicator && (
          <>
            <DropdownMenuSeparator />
            <div className="flex items-center gap-2 px-2 py-1.5 text-[11px] text-muted-foreground">
              <span
                className={`inline-block size-2 rounded-full ${connected ? 'bg-green-500' : 'bg-red-500 animate-pulse'}`}
              />
              {connected ? 'Connected to server' : 'Reconnecting to server'}
            </div>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
