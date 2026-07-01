import { useCallback, useState } from 'react'
import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  CloudUpload,
  MoreHorizontal,
  Plus,
  Server,
} from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { convertFileSrc, convertProjectFileSrc } from '@/lib/transport'
import { cn } from '@/lib/utils'
import { dismissibleToast } from '@/lib/dismissible-toast'
import type { Project } from '@/types/projects'
import { isBaseSession } from '@/types/projects'
import { useProjectsStore } from '@/store/projects-store'
import { useChatStore } from '@/store/chat-store'
import { useUIStore } from '@/store/ui-store'
import { useIsMobile } from '@/hooks/use-mobile'
import { useRemotePicker } from '@/hooks/useRemotePicker'
import {
  cloneProjectToServer,
  useWorktrees,
  useAppDataDir,
  projectsQueryKeys,
} from '@/services/projects'
import { useRemoteServers } from '@/services/remote-servers'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  useFetchWorktreesStatus,
  useGitStatus,
  gitPush,
  fetchWorktreesStatus,
  performGitPull,
} from '@/services/git-status'
import { NewIssuesBadge } from '@/components/shared/NewIssuesBadge'
import { OpenPRsBadge } from '@/components/shared/OpenPRsBadge'
import { FailedRunsBadge } from '@/components/shared/FailedRunsBadge'
import { SecurityAlertsBadge } from '@/components/shared/SecurityAlertsBadge'
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from '@/components/ui/tooltip'
import { WorktreeList } from './WorktreeList'
import { ProjectContextMenu } from './ProjectContextMenu'
import { RunWhereModal } from '@/components/remote/RunWhereModal'

interface ProjectTreeItemProps {
  project: Project
}

export function ProjectTreeItem({ project }: ProjectTreeItemProps) {
  const isMobile = useIsMobile()
  const {
    expandedProjectIds,
    selectedProjectId,
    selectProject,
    toggleProjectExpanded,
    openProjectSettings,
  } = useProjectsStore()
  const { data: worktrees = [] } = useWorktrees(project.id)
  const { data: appDataDir = '' } = useAppDataDir()
  const { data: remoteServers = [] } = useRemoteServers()
  const queryClient = useQueryClient()

  // Provisioned servers available for cloning
  const cloneableServers = remoteServers.filter(s => s.http_token)
  // Servers this project is already cloned onto
  const clonedServerIds = new Set((project.remote_clones ?? []).map(c => c.server_id))

  const handleCloneToServer = useCallback(
    (serverId: string, serverName: string) => {
      const toastId = toast.loading(`Cloning to ${serverName}...`)
      cloneProjectToServer(project.id, serverId)
        .then(() => {
          queryClient.invalidateQueries({ queryKey: projectsQueryKeys.list() })
          toast.success(`Cloned to ${serverName}`, { id: toastId })
        })
        .catch((err: unknown) => {
          toast.error(`Clone failed: ${err}`, { id: toastId })
        })
    },
    [project.id, queryClient]
  )
  const hasWorktrees = worktrees.length > 0
  const isExpanded = hasWorktrees && expandedProjectIds.has(project.id)
  const setNewWorktreeModalOpen = useUIStore(
    state => state.setNewWorktreeModalOpen
  )

  const avatarKey = project.avatar_path ?? project.default_avatar_path ?? null

  // Track image load errors to fall back to letter avatar
  // Use avatar key to reset error state when it changes
  const [imgErrorKey, setImgErrorKey] = useState<string | null>(null)
  const imgError = imgErrorKey === avatarKey

  // Build avatar URL from relative path
  const avatarUrl =
    project.avatar_path && appDataDir && !imgError
      ? convertFileSrc(`${appDataDir}/${project.avatar_path}`)
      : project.default_avatar_path && !imgError
        ? convertProjectFileSrc(project.default_avatar_path)
        : null

  // Fetch git status for all worktrees when project is expanded
  useFetchWorktreesStatus(project.id, isExpanded)

  // Check if base session exists
  const hasBaseSession = worktrees.some(w => isBaseSession(w))

  // Get base branch status from any worktree (all have it)
  const firstWorktree = worktrees[0]
  const { data: gitStatus } = useGitStatus(firstWorktree?.id ?? null)

  // Only show on project line when no base session
  const baseBranchBehindCount = !hasBaseSession
    ? (gitStatus?.base_branch_behind_count ??
      firstWorktree?.cached_base_branch_behind_count ??
      0)
    : 0
  const baseBranchAheadCount = !hasBaseSession
    ? (gitStatus?.base_branch_ahead_count ??
      firstWorktree?.cached_base_branch_ahead_count ??
      0)
    : 0

  // Get chat store state
  const activeWorktreeId = useChatStore(state => state.activeWorktreeId)
  const clearActiveWorktree = useChatStore(state => state.clearActiveWorktree)

  // Project is only selected if it's the selected project AND no worktree is active
  const isSelected = selectedProjectId === project.id && !activeWorktreeId
  const showStatusBadges = !isMobile && (isExpanded || isSelected)

  const handleClick = useCallback(() => {
    selectProject(project.id)
    // Clear active worktree so ChatWindow shows project canvas view
    clearActiveWorktree()
    // Close sidebar on mobile after navigation
    if (isMobile) {
      useUIStore.getState().setLeftSidebarVisible(false)
    }
  }, [isMobile, project.id, selectProject, clearActiveWorktree])

  const handleChevronClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      toggleProjectExpanded(project.id)
    },
    [project.id, toggleProjectExpanded]
  )

  const [newWorktreeWhereOpen, setNewWorktreeWhereOpen] = useState(false)

  const handleAddWorktree = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      selectProject(project.id)
      if ((project.remote_clones?.length ?? 0) > 0) {
        setNewWorktreeWhereOpen(true)
      } else {
        setNewWorktreeModalOpen(true)
      }
    },
    [project.id, project.remote_clones, selectProject, setNewWorktreeModalOpen]
  )

  const handleBasePull = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation()
      await performGitPull({
        worktreeId: '',
        worktreePath: project.path,
        baseBranch: project.default_branch,
        projectId: project.id,
      })
    },
    [project.id, project.path, project.default_branch]
  )

  const pickRemoteOrRun = useRemotePicker(project.path)

  const handleBasePush = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      pickRemoteOrRun(async remote => {
        const opToast = dismissibleToast.loading('Pushing changes...')
        try {
          await gitPush(project.path, undefined, remote)
          fetchWorktreesStatus(project.id)
          opToast.success('Changes pushed')
        } catch (error) {
          opToast.error(`Push failed: ${error}`)
        }
      })
    },
    [pickRemoteOrRun, project.id, project.path]
  )

  return (
    <>
    <ProjectContextMenu project={project}>
      <div>
        {/* Project Row */}
        <div
          className={cn(
            'group relative flex cursor-pointer items-center gap-1.5 px-2 py-1.5 overflow-hidden transition-colors duration-150',
            isSelected
              ? 'bg-primary/10 text-foreground before:absolute before:left-0 before:top-0 before:h-full before:w-[3px] before:bg-primary'
              : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'
          )}
          onClick={handleClick}
        >
          {/* Avatar */}
          {avatarUrl ? (
            <img
              src={avatarUrl}
              alt={project.name}
              className="size-4 shrink-0 rounded object-cover"
              onError={() => setImgErrorKey(avatarKey)}
            />
          ) : (
            <div className="flex size-4 shrink-0 items-center justify-center rounded bg-muted-foreground/20">
              <span className="text-[10px] font-medium uppercase">
                {project.name[0]}
              </span>
            </div>
          )}

          {/* Name + Chevron */}
          <span className="flex flex-1 items-center gap-0.5 truncate text-sm">
            <span className="truncate">{project.name}</span>
            {hasWorktrees && (
              <button
                className={cn(
                  'flex size-4 shrink-0 items-center justify-center rounded transition-opacity hover:bg-accent-foreground/10',
                  isMobile
                    ? 'opacity-70'
                    : 'opacity-0 group-hover:opacity-50 hover:!opacity-100'
                )}
                onClick={handleChevronClick}
              >
                <ChevronDown
                  className={cn(
                    'size-3 transition-transform',
                    isExpanded && 'rotate-180'
                  )}
                />
              </button>
            )}
          </span>

          {/* Base branch pull/push indicators (when no base session) */}
          {baseBranchBehindCount > 0 && (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={handleBasePull}
                  className="shrink-0 rounded bg-primary/10 px-1.5 py-0.5 text-[11px] font-medium text-primary transition-colors hover:bg-primary/20"
                >
                  <span className="flex items-center gap-0.5">
                    <ArrowDown className="h-3 w-3" />
                    {baseBranchBehindCount}
                  </span>
                </button>
              </TooltipTrigger>
              <TooltipContent>{`Pull ${baseBranchBehindCount} commit${baseBranchBehindCount > 1 ? 's' : ''} on ${project.default_branch}`}</TooltipContent>
            </Tooltip>
          )}
          {baseBranchAheadCount > 0 && (
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  onClick={handleBasePush}
                  className="shrink-0 rounded bg-orange-500/10 px-1.5 py-0.5 text-[11px] font-medium text-orange-500 transition-colors hover:bg-orange-500/20"
                >
                  <span className="flex items-center gap-0.5">
                    <ArrowUp className="h-3 w-3" />
                    {baseBranchAheadCount}
                  </span>
                </button>
              </TooltipTrigger>
              <TooltipContent>{`Push ${baseBranchAheadCount} commit${baseBranchAheadCount > 1 ? 's' : ''} on ${project.default_branch}`}</TooltipContent>
            </Tooltip>
          )}

          {showStatusBadges && (
            <div className="hidden items-center gap-1 sm:flex">
              <NewIssuesBadge
                projectPath={project.path}
                projectId={project.id}
              />
              <OpenPRsBadge projectPath={project.path} projectId={project.id} />
              <SecurityAlertsBadge
                projectPath={project.path}
                projectId={project.id}
              />
              <FailedRunsBadge projectPath={project.path} />
            </div>
          )}

          {/* Clone to remote server */}
          {cloneableServers.length > 0 && (() => {
            const singleServer = cloneableServers[0]
            if (cloneableServers.length === 1 && singleServer && !clonedServerIds.has(singleServer.id)) {
              return (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      onClick={e => {
                        e.stopPropagation()
                        handleCloneToServer(singleServer.id, singleServer.name)
                      }}
                      className="flex size-4 shrink-0 items-center justify-center rounded opacity-0 group-hover:opacity-50 hover:!opacity-100 hover:bg-accent-foreground/10"
                    >
                      <CloudUpload className="size-3.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent>Clone to {singleServer.name}</TooltipContent>
                </Tooltip>
              )
            }
            return (
              <DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <button
                        onClick={e => e.stopPropagation()}
                        className="flex size-4 shrink-0 items-center justify-center rounded opacity-0 group-hover:opacity-50 hover:!opacity-100 hover:bg-accent-foreground/10"
                      >
                        <CloudUpload className="size-3.5" />
                      </button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent>
                    {clonedServerIds.size > 0
                      ? `Cloned to: ${cloneableServers.filter(s => clonedServerIds.has(s.id)).map(s => s.name).join(', ')}`
                      : 'Clone to remote server'}
                  </TooltipContent>
                </Tooltip>
                <DropdownMenuContent align="end" onClick={e => e.stopPropagation()}>
                  {cloneableServers.map(server => (
                    <DropdownMenuItem
                      key={server.id}
                      disabled={clonedServerIds.has(server.id)}
                      onClick={() => handleCloneToServer(server.id, server.name)}
                    >
                      <Server className="mr-2 size-3.5" />
                      {server.name}
                      {clonedServerIds.has(server.id) && (
                        <span className="ml-auto text-xs text-muted-foreground">cloned</span>
                      )}
                    </DropdownMenuItem>
                  ))}
                  {cloneableServers.every(s => clonedServerIds.has(s.id)) && (
                    <>
                      <DropdownMenuSeparator />
                      <div className="px-2 py-1.5 text-xs text-muted-foreground">
                        Already cloned to all servers
                      </div>
                    </>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            )
          })()}

          {/* Settings */}
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={e => {
                  e.stopPropagation()
                  openProjectSettings(project.id)
                }}
                className="flex size-4 shrink-0 items-center justify-center rounded opacity-50 hover:bg-accent-foreground/10 hover:opacity-100"
              >
                <MoreHorizontal className="size-3.5" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Project settings</TooltipContent>
          </Tooltip>

          {/* Add Worktree */}
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                onClick={handleAddWorktree}
                className="flex size-4 shrink-0 items-center justify-center rounded opacity-50 hover:bg-accent-foreground/10 hover:opacity-100"
              >
                <Plus className="size-3.5" />
              </button>
            </TooltipTrigger>
            <TooltipContent>New worktree</TooltipContent>
          </Tooltip>
        </div>

        {/* Worktrees */}
        {isExpanded && (
          <WorktreeList
            projectId={project.id}
            projectPath={project.path}
            worktrees={worktrees}
            defaultBranch={project.default_branch}
          />
        )}
      </div>
    </ProjectContextMenu>
    <RunWhereModal
      open={newWorktreeWhereOpen}
      onOpenChange={setNewWorktreeWhereOpen}
      onSelect={serverId => setNewWorktreeModalOpen(true, serverId)}
      projectName={project.name}
      clonedServerIds={(project.remote_clones ?? []).map(c => c.server_id)}
    />
    </>
  )
}
