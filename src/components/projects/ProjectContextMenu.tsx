import {
  ArrowUpToLine,
  Code,
  ExternalLink,
  Folder,
  FolderOpen,
  Home,
  Plus,
  Server,
  Settings,
  Terminal,
  Trash2,
} from 'lucide-react'
import { useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { isBaseSession, type Project } from '@/types/projects'
import {
  cloneProjectToServer,
  useCreateBaseSession,
  useMoveItem,
  useOpenProjectOnGitHub,
  useOpenProjectWorktreesFolder,
  useOpenWorktreeInEditor,
  useOpenWorktreeInFinder,
  useOpenWorktreeInTerminal,
  useRemoveProject,
  useWorktrees,
} from '@/services/projects'
import { projectsQueryKeys } from '@/services/projects'
import { usePreferences } from '@/services/preferences'
import { useProjectsStore } from '@/store/projects-store'
import { useUIStore } from '@/store/ui-store'
import { getEditorLabel, getTerminalLabel } from '@/types/preferences'
import { getFileManagerName } from '@/lib/platform'
import { isNativeApp } from '@/lib/environment'
import { useRemoteServers } from '@/services/remote-servers'
import { RunWhereModal } from '@/components/remote/RunWhereModal'

interface ProjectContextMenuProps {
  project: Project
  children: React.ReactNode
}

export function ProjectContextMenu({
  project,
  children,
}: ProjectContextMenuProps) {
  const [runWhereOpen, setRunWhereOpen] = useState(false)
  const createBaseSession = useCreateBaseSession()
  const moveItem = useMoveItem()
  const removeProject = useRemoveProject()
  const openOnGitHub = useOpenProjectOnGitHub()
  const openInFinder = useOpenWorktreeInFinder()
  const openWorktreesFolder = useOpenProjectWorktreesFolder()
  const openInTerminal = useOpenWorktreeInTerminal()
  const openInEditor = useOpenWorktreeInEditor()
  const { data: worktrees = [] } = useWorktrees(project.id)
  const { data: preferences } = usePreferences()
  const { openProjectSettings, selectProject } = useProjectsStore()
  const setNewWorktreeModalOpen = useUIStore(
    state => state.setNewWorktreeModalOpen
  )
  const queryClient = useQueryClient()
  const { data: remoteServers = [] } = useRemoteServers()

  // Only connected + provisioned servers can receive clones
  const cloneableServers = remoteServers.filter(
    s => s.status === 'connected' && s.http_token
  )

  // Check if base session already exists
  const existingBaseSession = worktrees.find(isBaseSession)
  const isNested = project.parent_id !== undefined

  const handleOpenInFinder = () => {
    openInFinder.mutate(project.path)
  }

  const handleOpenWorktreesFolder = () => {
    openWorktreesFolder.mutate(project.id)
  }

  const handleOpenInTerminal = () => {
    openInTerminal.mutate({
      worktreePath: project.path,
      terminal: preferences?.terminal,
    })
  }

  const handleOpenInEditor = () => {
    openInEditor.mutate({
      worktreePath: project.path,
      editor: preferences?.editor,
    })
  }

  const [newWorktreeWhereOpen, setNewWorktreeWhereOpen] = useState(false)

  const handleNewWorktree = () => {
    selectProject(project.id)
    if ((project.remote_clones?.length ?? 0) > 0) {
      setNewWorktreeWhereOpen(true)
    } else {
      setNewWorktreeModalOpen(true)
    }
  }

  const handleNewBaseSession = () => {
    if (cloneableServers.length > 0) {
      setRunWhereOpen(true)
    } else {
      createBaseSession.mutate({ projectId: project.id })
    }
  }

  const handleRunWhereSelect = (serverId: string | null) => {
    createBaseSession.mutate({ projectId: project.id, serverId: serverId ?? undefined })
  }

  const handleRemoveProject = () => {
    removeProject.mutate(project.id)
  }

  const handleOpenOnGitHub = () => {
    openOnGitHub.mutate(project.id)
  }

  const handleMoveToRoot = () => {
    moveItem.mutate({ itemId: project.id, newParentId: undefined })
  }

  const handleOpenSettings = () => {
    openProjectSettings(project.id)
  }

  const handleCloneToServer = (serverId: string, serverName: string) => {
    const toastId = toast.loading(`Cloning to ${serverName}...`)
    cloneProjectToServer(project.id, serverId)
      .then(() => {
        queryClient.invalidateQueries({ queryKey: projectsQueryKeys.list() })
        toast.success(`Cloned to ${serverName}`, { id: toastId })
      })
      .catch((error: unknown) => {
        toast.error(`Clone failed: ${error}`, { id: toastId })
      })
  }

  return (
    <>
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-64">
        <ContextMenuItem onClick={handleOpenSettings}>
          <Settings className="mr-2 h-4 w-4" />
          Project Settings
        </ContextMenuItem>

        {isNested && (
          <ContextMenuItem onClick={handleMoveToRoot}>
            <ArrowUpToLine className="mr-2 h-4 w-4" />
            Move to Root
          </ContextMenuItem>
        )}

        <ContextMenuSeparator />

        <ContextMenuItem onClick={handleNewBaseSession}>
          <Home className="mr-2 h-4 w-4" />
          {existingBaseSession ? 'Open Base Session' : 'New Base Session'}
        </ContextMenuItem>

        <ContextMenuItem onClick={handleNewWorktree}>
          <Plus className="mr-2 h-4 w-4" />
          New Worktree
        </ContextMenuItem>

        <ContextMenuSeparator />

        <ContextMenuItem onClick={handleOpenInEditor}>
          <Code className="mr-2 h-4 w-4" />
          Open in {getEditorLabel(preferences?.editor)}
        </ContextMenuItem>

        {isNativeApp() && (
          <ContextMenuItem onClick={handleOpenInFinder}>
            <FolderOpen className="mr-2 h-4 w-4" />
            Open in {getFileManagerName()}
          </ContextMenuItem>
        )}

        <ContextMenuItem onClick={handleOpenInTerminal}>
          <Terminal className="mr-2 h-4 w-4" />
          Open in {getTerminalLabel(preferences?.terminal)}
        </ContextMenuItem>

        <ContextMenuSeparator />

        <ContextMenuItem onClick={handleOpenWorktreesFolder}>
          <Folder className="mr-2 h-4 w-4" />
          Open Worktrees Folder
        </ContextMenuItem>

        <ContextMenuItem onClick={handleOpenOnGitHub}>
          <ExternalLink className="mr-2 h-4 w-4" />
          Open on GitHub
        </ContextMenuItem>

        {cloneableServers.length > 0 && (
          <>
            <ContextMenuSeparator />
            <ContextMenuSub>
              <ContextMenuSubTrigger>
                <Server className="mr-2 h-4 w-4" />
                Clone to remote
              </ContextMenuSubTrigger>
              <ContextMenuSubContent className="w-48">
                {cloneableServers.map(server => (
                  <ContextMenuItem
                    key={server.id}
                    onClick={() => handleCloneToServer(server.id, server.name)}
                  >
                    <Server className="mr-2 h-3.5 w-3.5 text-muted-foreground" />
                    {server.name}
                  </ContextMenuItem>
                ))}
              </ContextMenuSubContent>
            </ContextMenuSub>
          </>
        )}

        <ContextMenuSeparator />

        <ContextMenuItem
          variant="destructive"
          onClick={handleRemoveProject}
          disabled={worktrees.length > 0}
          className="whitespace-nowrap"
        >
          <Trash2 className="mr-2 h-4 w-4 shrink-0" />
          Remove Project
          {worktrees.length > 0 && (
            <span className="ml-auto text-xs opacity-60 shrink-0">
              ({worktrees.length} worktrees)
            </span>
          )}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
    <RunWhereModal
      open={runWhereOpen}
      onOpenChange={setRunWhereOpen}
      onSelect={handleRunWhereSelect}
      projectName={project.name}
      clonedServerIds={(project.remote_clones ?? []).map(c => c.server_id)}
    />
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
