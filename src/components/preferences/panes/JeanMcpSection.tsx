import React, { useCallback, useEffect, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { CheckCircle, Copy, Loader2, PlugZap, XCircle } from 'lucide-react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { copyToClipboard } from '@/lib/clipboard'
import { cn } from '@/lib/utils'
import { invoke, listen } from '@/lib/transport'
import { useInstalledBackends } from '@/hooks/useInstalledBackends'
import { invalidateAllMcpServers } from '@/services/mcp'
import { usePatchPreferences, usePreferences } from '@/services/preferences'
import type { CliBackend } from '@/types/preferences'
import { SettingsSection } from '../SettingsSection'

interface JeanMcpSnippet {
  enabled: boolean
  serverRunning: boolean
  mode: 'dev' | 'prod'
  serverName: string
  url: string | null
  token: string | null
  claude: string | null
  cursor: string | null
  codexToml: string | null
  opencodeJson: string | null
}

interface JeanMcpInstallResult {
  backend: CliBackend
  status: 'installed' | 'error'
  path: string | null
  backupPath: string | null
  serverName: string
  mode: 'dev' | 'prod'
  message: string
}

type InstallState = 'idle' | 'installing' | 'waiting' | 'success' | 'error'

const INSTALLABLE_BACKENDS: CliBackend[] = [
  'claude',
  'codex',
  'opencode',
  'cursor',
]

function installButtonContent(state: InstallState, message: string) {
  switch (state) {
    case 'installing':
      return (
        <>
          <Loader2 className="size-3.5 animate-spin" />
          <span>Adding...</span>
        </>
      )
    case 'waiting':
      return (
        <>
          <Loader2 className="size-3.5 animate-spin" />
          <span className="truncate">Waiting for MCP...</span>
        </>
      )
    case 'success':
      return (
        <>
          <CheckCircle className="size-3.5" />
          <span className="truncate">{message || 'Added'}</span>
        </>
      )
    case 'error':
      return (
        <>
          <XCircle className="size-3.5" />
          <span className="truncate">{message}</span>
        </>
      )
    default:
      return null
  }
}

export const JeanMcpSection: React.FC = () => {
  const { data: preferences } = usePreferences()
  const patchPreferences = usePatchPreferences()
  const { installedBackends } = useInstalledBackends()
  const queryClient = useQueryClient()
  const [installState, setInstallState] = useState<InstallState>('idle')
  const [installMessage, setInstallMessage] = useState('')
  const [showInstallChoice, setShowInstallChoice] = useState(false)

  const enabled = preferences?.jean_mcp_enabled ?? true
  const {
    data: snippet,
    refetch: refreshSnippet,
    isLoading: isSnippetLoading,
    isFetching: isSnippetFetching,
  } = useQuery<JeanMcpSnippet>({
    queryKey: ['jeanMcpSnippet', enabled],
    queryFn: () => invoke<JeanMcpSnippet>('get_jean_mcp_config_snippet'),
    enabled,
  })

  const serverRunning = snippet?.serverRunning ?? false
  const checkingServer =
    enabled && !snippet && (isSnippetLoading || isSnippetFetching)
  const modeLabel = (snippet?.mode ?? 'prod') === 'dev' ? 'Dev' : 'Prod'
  const installableBackends = installedBackends.filter(backend =>
    INSTALLABLE_BACKENDS.includes(backend)
  )

  const setTemporaryInstallState = useCallback(
    (state: InstallState, message = '') => {
      setInstallState(state)
      setInstallMessage(message)
      if (state === 'success' || state === 'error') {
        window.setTimeout(() => {
          setInstallState('idle')
          setInstallMessage('')
        }, 5000)
      }
    },
    []
  )

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let disposed = false
    listen('jean-mcp-socket-status', () => {
      queryClient.invalidateQueries({ queryKey: ['jeanMcpSnippet'] })
    }).then(fn => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [queryClient])

  const handleInstall = useCallback(
    async (assumeEnabled = false) => {
      if ((!enabled && !assumeEnabled) || !serverRunning) {
        setTemporaryInstallState('error', 'Enable Jean MCP first')
        return
      }
      if (installableBackends.length === 0) {
        setTemporaryInstallState(
          'error',
          'Install Claude, Codex, OpenCode, or Cursor first'
        )
        return
      }

      setTemporaryInstallState('installing')
      try {
        const results = await invoke<JeanMcpInstallResult[]>(
          'install_jean_mcp_config',
          {
            backends: installableBackends,
            mode: 'current',
          }
        )
        const successes = results.filter(r => r.status === 'installed')
        const failures = results.filter(r => r.status === 'error')
        invalidateAllMcpServers(undefined, installableBackends)
        await refreshSnippet()
        setTemporaryInstallState(
          failures.length > 0 ? 'error' : 'success',
          failures.length > 0
            ? `Added ${successes.length}/${results.length}; ${failures.length} failed`
            : 'Added'
        )
      } catch (e) {
        setTemporaryInstallState('error', 'Failed to add Jean MCP')
        console.error('Failed to add Jean MCP config', e)
      }
    },
    [
      enabled,
      installableBackends,
      refreshSnippet,
      serverRunning,
      setTemporaryInstallState,
    ]
  )

  useEffect(() => {
    if (installState === 'waiting' && enabled && serverRunning) {
      handleInstall(true)
    }
  }, [enabled, handleInstall, installState, serverRunning])

  const handleEnabledChange = (checked: boolean) => {
    patchPreferences.mutate({ jean_mcp_enabled: checked })
    setTemporaryInstallState('idle')
    setShowInstallChoice(checked)
  }

  const handleAddAutomatically = () => {
    setShowInstallChoice(false)
    if (serverRunning) handleInstall(true)
    else setTemporaryInstallState('waiting')
  }

  const handleCopy = (label: string, content: string | null) => {
    if (!content) {
      toast.error(`No ${label} snippet available — enable Jean MCP first`)
      return
    }
    copyToClipboard(content)
    toast.success(`${label} snippet copied`)
  }

  const transientButton = installButtonContent(installState, installMessage)

  return (
    <>
      <SettingsSection title="Jean MCP Server" anchorId="pref-mcp-section-jean">
        <p className="text-sm text-muted-foreground">
          Expose Jean&apos;s own commands over MCP so spawned local CLIs can
          call back into Jean (create worktrees, list GitHub issues, send chat
          messages, etc).
        </p>
        <div className="flex items-center gap-3 rounded-md border px-4 py-3">
          <Switch
            id="jean-mcp-enabled"
            checked={enabled}
            onCheckedChange={handleEnabledChange}
          />
          <Label htmlFor="jean-mcp-enabled" className="flex-1 cursor-pointer">
            Enable Jean MCP
          </Label>
          {checkingServer && (
            <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Loader2 className="size-3.5 animate-spin" />
              Checking MCP socket…
            </span>
          )}
          {!checkingServer && !serverRunning && enabled && (
            <span className="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400">
              <PlugZap className="size-3.5" />
              MCP socket not running
            </span>
          )}
        </div>

        {enabled && (
          <>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-1.5">
                <Label htmlFor="jean-mcp-max-depth" className="text-xs">
                  Max recursion depth
                </Label>
                <Input
                  id="jean-mcp-max-depth"
                  type="number"
                  min={0}
                  max={10}
                  value={preferences?.jean_mcp_max_depth ?? 3}
                  onChange={e =>
                    patchPreferences.mutate({
                      jean_mcp_max_depth: Math.max(
                        0,
                        Math.min(10, Number(e.target.value) || 0)
                      ),
                    })
                  }
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="jean-mcp-rate-limit" className="text-xs">
                  Spawn rate limit (per minute)
                </Label>
                <Input
                  id="jean-mcp-rate-limit"
                  type="number"
                  min={0}
                  max={1000}
                  value={preferences?.jean_mcp_rate_limit_per_minute ?? 20}
                  onChange={e =>
                    patchPreferences.mutate({
                      jean_mcp_rate_limit_per_minute: Math.max(
                        0,
                        Math.min(1000, Number(e.target.value) || 0)
                      ),
                    })
                  }
                />
              </div>
            </div>

            <div className="space-y-2 rounded-md border px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <div className="space-y-1">
                  <Label className="text-sm font-medium">
                    One-click config install
                  </Label>
                  <p className="text-xs text-muted-foreground">
                    Safely merges Jean MCP config into the CLI user configs.
                  </p>
                </div>
                <Button
                  size="sm"
                  onClick={() => handleInstall()}
                  disabled={
                    installState === 'installing' ||
                    installState === 'success' ||
                    !serverRunning ||
                    installableBackends.length === 0
                  }
                  className={cn(
                    'min-w-[11rem] max-w-[18rem]',
                    installState === 'success' &&
                      'border-green-600 bg-green-600 text-white hover:bg-green-700'
                  )}
                  aria-live="polite"
                  title={installMessage}
                >
                  {transientButton ?? (
                    <span>Add current Jean MCP ({modeLabel})</span>
                  )}
                </Button>
              </div>
            </div>

            <div className="space-y-2">
              <Label className="text-xs uppercase tracking-wider text-muted-foreground">
                Manual setup snippets
              </Label>
              <div className="grid grid-cols-2 gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => handleCopy('Claude', snippet?.claude ?? null)}
                >
                  <Copy className="mr-2 size-3.5" />
                  Claude (~/.claude.json)
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => handleCopy('Cursor', snippet?.cursor ?? null)}
                >
                  <Copy className="mr-2 size-3.5" />
                  Cursor (~/.cursor/mcp.json)
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    handleCopy('Codex', snippet?.codexToml ?? null)
                  }
                >
                  <Copy className="mr-2 size-3.5" />
                  Codex (~/.codex/config.toml)
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    handleCopy('OpenCode', snippet?.opencodeJson ?? null)
                  }
                >
                  <Copy className="mr-2 size-3.5" />
                  OpenCode (~/.config/opencode/opencode.json)
                </Button>
              </div>
            </div>
          </>
        )}
      </SettingsSection>

      <AlertDialog open={showInstallChoice} onOpenChange={setShowInstallChoice}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Add Jean MCP to your CLI configs?
            </AlertDialogTitle>
            <AlertDialogDescription>
              Jean MCP is enabled. Jean can add it automatically to your
              installed CLI config files, or you can copy the manual snippets
              below.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Manual setup</AlertDialogCancel>
            <AlertDialogAction onClick={handleAddAutomatically}>
              Add automatically
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
