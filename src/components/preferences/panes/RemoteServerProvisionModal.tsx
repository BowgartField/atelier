import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from 'react'
import {
  CheckCircle2,
  CircleAlert,
  CircleDot,
  Loader2,
  ServerCog,
  ShieldCheck,
} from 'lucide-react'
import { invoke, listen } from '@/lib/transport'
import { toast } from 'sonner'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Checkbox } from '@/components/ui/checkbox'
import { ScrollArea } from '@/components/ui/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { cn } from '@/lib/utils'
import { FALLBACK_APP_VERSION } from '@/lib/app-version'
import { resolveDefaultProvisionVersion } from '@/lib/remote-versions'
import {
  useConnectRemoteServer,
  useProvisionRemoteServer,
  useRemoteJeanVersions,
} from '@/services/remote-servers'
import type {
  LocalToolStatus,
  RemoteProvisionLogLine,
  RemoteProvisionProgress,
  RemoteServerConfig,
  ToolsToInstall,
} from '@/types/remote'

interface ProvisionStep {
  stage: string
  label: string
  description: string
}

const BASE_PROVISION_STEPS: ProvisionStep[] = [
  {
    stage: 'preparing',
    label: 'Prepare server',
    description: 'Check sudo access and install runtime dependencies.',
  },
  {
    stage: 'detecting_architecture',
    label: 'Detect architecture',
    description: 'Read uname output and select the matching artifact.',
  },
  {
    stage: 'downloading_release',
    label: 'Download release',
    description: 'Fetch the signed Jean release manifest and artifact.',
  },
  {
    stage: 'uploading_artifact',
    label: 'Upload AppImage',
    description: 'Copy the extracted Jean binary to the remote host.',
  },
  {
    stage: 'verifying_service',
    label: 'Verify service',
    description: 'Start the systemd unit and confirm it is active.',
  },
]

const CONNECTING_STEP: ProvisionStep = {
  stage: 'connecting',
  label: 'Connect',
  description: 'Open the SSH tunnel and register the remote backend.',
}

const CLAUDE_CLI_STEP: ProvisionStep = {
  stage: 'installing_claude_cli',
  label: 'Install Claude CLI',
  description: 'Download and install the Claude CLI binary.',
}

const GH_CLI_STEP: ProvisionStep = {
  stage: 'installing_gh_cli',
  label: 'Install GitHub CLI',
  description: 'Install gh from the official package repository.',
}

const COMPLETE_STEP: ProvisionStep = {
  stage: 'complete',
  label: 'Ready',
  description: 'Remote Jean is available through the SSH tunnel.',
}

function buildSteps(tools: ToolsToInstall): ProvisionStep[] {
  return [
    ...BASE_PROVISION_STEPS,
    CONNECTING_STEP,
    ...(tools.claudeCli ? [CLAUDE_CLI_STEP] : []),
    ...(tools.ghCli ? [GH_CLI_STEP] : []),
    COMPLETE_STEP,
  ]
}

interface RemoteServerProvisionModalProps {
  open: boolean
  server: RemoteServerConfig | null
  onOpenChange: (open: boolean) => void
}

function stageIndex(steps: ProvisionStep[], stage?: string | null): number {
  if (!stage) return -1
  return steps.findIndex(step => step.stage === stage)
}

function logClassName(stream: RemoteProvisionLogLine['stream']) {
  if (stream === 'stderr') return 'text-destructive'
  if (stream === 'system') return 'text-muted-foreground'
  return 'text-foreground'
}

function ProvisionLogPanel({
  logs,
  viewportRef,
}: {
  logs: RemoteProvisionLogLine[]
  viewportRef: RefObject<HTMLDivElement | null>
}) {
  return (
    <ScrollArea
      className="flex-1 min-h-0 rounded-xl border bg-muted/10"
      viewportRef={viewportRef}
    >
      <div className="space-y-1 p-3 font-mono text-[11px] leading-5">
        {logs.length === 0 ? (
          <p className="text-muted-foreground">
            Provisioning output will appear here.
          </p>
        ) : (
          logs.map((log, index) => (
            <div key={`${index}-${log.stream}-${log.line}`} className="flex gap-2">
              <span className="shrink-0 text-muted-foreground">
                {log.stream === 'stderr'
                  ? '[err]'
                  : log.stream === 'system'
                    ? '[sys]'
                    : '[out]'}
              </span>
              <span className={cn('whitespace-pre-wrap break-words', logClassName(log.stream))}>
                {log.line}
              </span>
            </div>
          ))
        )}
      </div>
    </ScrollArea>
  )
}

function StepRail({
  progress,
  steps,
  running,
  isError,
}: {
  progress: RemoteProvisionProgress | null
  steps: ProvisionStep[]
  running: boolean
  isError: boolean
}) {
  const activeIndex = stageIndex(steps, progress?.stage)
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {steps.map((step, index) => {
        const completed = activeIndex > index || progress?.stage === 'complete'
        const errored = isError && activeIndex === index
        const active = running && activeIndex === index && !errored
        return (
          <div
            key={step.stage}
            className={cn(
              'rounded-xl border p-3 transition-colors',
              completed && 'border-emerald-500/25 bg-emerald-500/5',
              active && 'border-sky-500/25 bg-sky-500/5',
              errored && 'border-destructive/25 bg-destructive/5'
            )}
          >
            <div className="flex items-start gap-3">
              <div
                className={cn(
                  'mt-0.5 grid size-6 shrink-0 place-items-center rounded-full border',
                  completed &&
                    'border-emerald-500/30 bg-emerald-500/10 text-emerald-500',
                  active &&
                    'border-sky-500/30 bg-sky-500/10 text-sky-500',
                  errored &&
                    'border-destructive/30 bg-destructive/10 text-destructive',
                  !completed &&
                    !active &&
                    !errored &&
                    'border-border bg-background text-muted-foreground'
                )}
              >
                {errored ? (
                  <CircleAlert className="size-3.5" />
                ) : completed ? (
                  <CheckCircle2 className="size-3.5" />
                ) : active ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <CircleDot className="size-3.5" />
                )}
              </div>
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <p className="text-sm font-medium">{step.label}</p>
                  {active && (
                    <Badge variant="outline" className="h-5 border-sky-500/25 px-1.5 text-[10px] text-sky-600 dark:text-sky-400">
                      Running
                    </Badge>
                  )}
                  {errored && (
                    <Badge variant="outline" className="h-5 border-destructive/25 px-1.5 text-[10px] text-destructive">
                      Failed
                    </Badge>
                  )}
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                  {step.description}
                </p>
              </div>
            </div>
          </div>
        )
      })}
    </div>
  )
}

export function RemoteServerProvisionModal({
  open,
  server,
  onOpenChange,
}: RemoteServerProvisionModalProps) {
  const provisionServer = useProvisionRemoteServer()
  const connectServer = useConnectRemoteServer()
  const { data: availableVersions } = useRemoteJeanVersions(open)
  const [progress, setProgress] = useState<RemoteProvisionProgress | null>(null)
  const [logs, setLogs] = useState<RemoteProvisionLogLine[]>([])
  const [error, setError] = useState<string | null>(null)
  const [completedVersion, setCompletedVersion] = useState<string | null>(null)
  const [running, setRunning] = useState(false)
  const [tools, setTools] = useState<ToolsToInstall>({ claudeCli: false, ghCli: false })
  const [selectedVersion, setSelectedVersion] = useState(FALLBACK_APP_VERSION)
  const initializedRef = useRef(false)
  const versionTouchedRef = useRef(false)
  const logsViewportRef = useRef<HTMLDivElement | null>(null)

  const serverId = server?.id ?? null

  const versionOptions = useMemo(() => {
    const versions = (availableVersions ?? []).map(entry => entry.version)
    return versions.includes(FALLBACK_APP_VERSION)
      ? versions
      : [FALLBACK_APP_VERSION, ...versions]
  }, [availableVersions])

  const desktopVersionPublished = (availableVersions ?? []).some(
    entry => entry.version === FALLBACK_APP_VERSION
  )
  const defaultVersion = useMemo(
    () => resolveDefaultProvisionVersion(availableVersions),
    [availableVersions]
  )

  // Once releases load, default to an installable version: the desktop's own
  // version if it's published, otherwise the latest release (dev builds run
  // ahead of the next tag, which has no release artifact yet).
  useEffect(() => {
    if (!open || versionTouchedRef.current) return
    setSelectedVersion(defaultVersion)
  }, [open, defaultVersion])

  const resetState = useCallback(() => {
    setProgress(null)
    setLogs([])
    setError(null)
    setCompletedVersion(null)
    setRunning(false)
    setSelectedVersion(FALLBACK_APP_VERSION)
    versionTouchedRef.current = false
    initializedRef.current = false
  }, [])

  // Load local tool status to set default checkboxes
  useEffect(() => {
    if (!open) return
    invoke<LocalToolStatus>('get_local_tool_status', {})
      .then(status => {
        setTools({ claudeCli: status.claude_cli, ghCli: status.gh_cli })
      })
      .catch(() => {/* ignore — defaults stay false */})
  }, [open])

  useEffect(() => {
    if (!open) {
      resetState()
      return
    }
    resetState()
  }, [open, serverId, resetState])

  useEffect(() => {
    if (!open || !serverId) return

    let unlistenProgress: (() => void) | null = null
    let unlistenLog: (() => void) | null = null
    let cancelled = false

    listen<RemoteProvisionProgress>('remote-server:provision-progress', event => {
      if (event.payload.server_id !== serverId) return
      setProgress(event.payload)
      if (event.payload.stage === 'complete') {
        setRunning(false)
      }
    })
      .then(unlisten => {
        if (cancelled) {
          unlisten()
          return
        }
        unlistenProgress = unlisten
      })
      .catch(error => {
        console.error('Failed to listen for remote provision progress', error)
      })

    listen<RemoteProvisionLogLine>('remote-server:provision-log', event => {
      if (event.payload.server_id !== serverId) return
      setLogs(current => [...current, event.payload].slice(-300))
    })
      .then(unlisten => {
        if (cancelled) {
          unlisten()
          return
        }
        unlistenLog = unlisten
      })
      .catch(error => {
        console.error('Failed to listen for remote provision logs', error)
      })

    return () => {
      cancelled = true
      unlistenProgress?.()
      unlistenLog?.()
    }
  }, [open, serverId])

  useEffect(() => {
    const viewport = logsViewportRef.current
    if (!viewport) return
    viewport.scrollTop = viewport.scrollHeight
  }, [logs])

  const provisionSteps = useMemo(() => buildSteps(tools), [tools])

  const currentStep = useMemo(() => {
    if (progress?.stage === 'complete') return COMPLETE_STEP
    return provisionSteps.find(step => step.stage === progress?.stage) ?? null
  }, [progress?.stage, provisionSteps])

  const startProvisioning = useCallback(async () => {
    if (!server || running || initializedRef.current) return
    initializedRef.current = true
    setRunning(true)
    setError(null)
    setLogs([])
    setCompletedVersion(null)

    try {
      const result = await provisionServer.mutateAsync({
        serverId: server.id,
        version: selectedVersion,
      })

      // Open the SSH tunnel and register the remote WebSocket transport so
      // subsequent _backendHandle calls (CLI installs, session creation)
      // have somewhere to route to. Provisioning alone does not connect.
      setProgress({
        server_id: server.id,
        stage: 'connecting',
        message: 'Connecting to remote backend…',
        percent: 88,
      })
      await connectServer.mutateAsync(server.id)

      // Install selected CLIs after jean-server is running
      const installErrors: string[] = []

      if (tools.claudeCli) {
        setProgress({
          server_id: server.id,
          stage: 'installing_claude_cli',
          message: 'Installing Claude CLI…',
          percent: 90,
        })
        try {
          await invoke('install_claude_cli', { _backendHandle: server.id })
        } catch (e) {
          installErrors.push(`Claude CLI: ${String(e)}`)
        }
      }

      if (tools.ghCli) {
        setProgress({
          server_id: server.id,
          stage: 'installing_gh_cli',
          message: 'Installing GitHub CLI…',
          percent: tools.claudeCli ? 95 : 90,
        })
        try {
          await invoke('install_gh_on_remote', { serverId: server.id })
        } catch (e) {
          installErrors.push(`GitHub CLI: ${String(e)}`)
        }
      }

      setCompletedVersion(result.version)
      setProgress({
        server_id: server.id,
        stage: 'complete',
        message: `Jean ${result.version} is running`,
        percent: 100,
      })
      setRunning(false)

      if (installErrors.length > 0) {
        toast.warning(`Provisioned, but some CLIs failed to install:\n${installErrors.join('\n')}`)
      }
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause)
      setError(message)
      setRunning(false)
      toast.error(`Provisioning failed: ${message}`)
    }
  }, [connectServer, provisionServer, running, selectedVersion, server, tools])

  const isComplete = progress?.stage === 'complete' && !running && !error
  const isError = error != null

  return (
    <Dialog
      open={open}
      onOpenChange={nextOpen => {
        if (!nextOpen && !running) onOpenChange(false)
      }}
    >
      <DialogContent
        className="!w-screen !h-dvh !max-w-screen !max-h-none !rounded-none sm:!w-[calc(100vw-64px)] sm:!max-w-[calc(100vw-64px)] sm:!h-[calc(100vh-64px)] sm:!rounded-lg flex flex-col overflow-hidden"
        preventClose={running}
        showCloseButton={!running}
      >
        <DialogHeader className="shrink-0 pr-12 text-left">
          <DialogTitle className="flex items-center gap-2">
            <span className="grid size-8 place-items-center rounded-lg border bg-background text-sky-500">
              <ServerCog className="size-4" />
            </span>
            <span className="min-w-0 truncate">
              {server ? `Provision ${server.name}` : 'Provision remote server'}
            </span>
          </DialogTitle>
          <DialogDescription>
            Install Jean and its runtime dependencies on the remote host, then
            start the headless backend behind the SSH tunnel.
          </DialogDescription>
        </DialogHeader>

        <div className="flex min-h-0 flex-1 flex-col gap-4">
          {!server ? (
            <Alert variant="destructive">
              <CircleAlert />
              <AlertTitle>No server selected</AlertTitle>
              <AlertDescription>
                Pick a remote server before starting provisioning.
              </AlertDescription>
            </Alert>
          ) : (
            <>
              <div className="rounded-xl border bg-muted/10 p-4">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div>
                    <p className="text-sm font-medium">
                      {currentStep?.label ?? 'Ready to provision'}
                    </p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {progress?.message ??
                        'Jean will install Xvfb, fetch the signed Linux artifact, and register a systemd service.'}
                    </p>
                  </div>
                  <Badge
                    variant="outline"
                    className={cn(
                      'gap-1.5',
                      isComplete &&
                        'border-emerald-500/25 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400',
                      isError &&
                        'border-destructive/25 bg-destructive/10 text-destructive'
                    )}
                  >
                    {running ? (
                      <Loader2 className="size-3.5 animate-spin" />
                    ) : isComplete ? (
                      <CheckCircle2 className="size-3.5" />
                    ) : isError ? (
                      <CircleAlert className="size-3.5" />
                    ) : (
                      <ShieldCheck className="size-3.5" />
                    )}
                    {running
                      ? 'Provisioning'
                      : isComplete
                        ? 'Done'
                        : isError
                          ? 'Failed'
                          : 'Ready'}
                  </Badge>
                </div>
                <div className="mt-4 h-2 w-full overflow-hidden rounded-full bg-secondary">
                  <div
                    className="h-full rounded-full bg-primary transition-[width] duration-300"
                    style={{ width: `${progress?.percent ?? 0}%` }}
                  />
                </div>
              </div>

              {/* Version selection — compact row, shown before provisioning starts */}
              {!running && !isComplete && !isError && (
                <div className="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border bg-muted/10 px-4 py-3">
                  <p className="shrink-0 text-xs font-medium text-muted-foreground">
                    Jean version:
                  </p>
                  <Select
                    value={selectedVersion}
                    onValueChange={v => {
                      versionTouchedRef.current = true
                      setSelectedVersion(v)
                    }}
                  >
                    <SelectTrigger className="h-8 w-48 text-sm">
                      <SelectValue placeholder="Select version" />
                    </SelectTrigger>
                    <SelectContent>
                      {versionOptions.map(version => {
                        const isDesktop = version === FALLBACK_APP_VERSION
                        const suffix = isDesktop
                          ? desktopVersionPublished
                            ? ' (current)'
                            : ' (this build, unreleased)'
                          : version === defaultVersion
                            ? ' (latest release)'
                            : ''
                        return (
                          <SelectItem key={version} value={version}>
                            {version}
                            {suffix}
                          </SelectItem>
                        )
                      })}
                    </SelectContent>
                  </Select>
                  {selectedVersion !== FALLBACK_APP_VERSION && (
                    <p className="text-xs text-muted-foreground">
                      Packaged release builds require this to match the
                      desktop version ({FALLBACK_APP_VERSION}) to connect; dev
                      builds skip that check.
                    </p>
                  )}
                  {selectedVersion === FALLBACK_APP_VERSION &&
                    !desktopVersionPublished && (
                      <p className="text-xs text-amber-600 dark:text-amber-400">
                        This build has no published release yet — provisioning
                        will fail until v{FALLBACK_APP_VERSION} ships.
                      </p>
                    )}
                </div>
              )}

              {/* Tool selection — compact row, shown before provisioning starts */}
              {!running && !isComplete && !isError && (
                <div className="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border bg-muted/10 px-4 py-3">
                  <p className="shrink-0 text-xs font-medium text-muted-foreground">
                    Also install:
                  </p>
                  <label className="flex cursor-pointer items-center gap-2">
                    <Checkbox
                      checked={tools.claudeCli}
                      onCheckedChange={v => setTools(t => ({ ...t, claudeCli: !!v }))}
                    />
                    <span className="text-sm">Claude CLI</span>
                  </label>
                  <label className="flex cursor-pointer items-center gap-2">
                    <Checkbox
                      checked={tools.ghCli}
                      onCheckedChange={v => setTools(t => ({ ...t, ghCli: !!v }))}
                    />
                    <span className="text-sm">GitHub CLI</span>
                  </label>
                </div>
              )}

              <StepRail
                progress={progress}
                steps={provisionSteps}
                running={running}
                isError={isError}
              />

              <div className="min-h-0 flex-1 flex flex-col">
                <ProvisionLogPanel logs={logs} viewportRef={logsViewportRef} />
              </div>

              {isError && (
                <Alert variant="destructive">
                  <CircleAlert />
                  <AlertTitle>Provisioning failed</AlertTitle>
                  <AlertDescription>{error}</AlertDescription>
                </Alert>
              )}

              {isComplete && (
                <Alert className="border-emerald-500/20 bg-emerald-500/5">
                  <CheckCircle2 className="text-emerald-500" />
                  <AlertTitle>Jean is running</AlertTitle>
                  <AlertDescription>
                    {completedVersion
                      ? `Version ${completedVersion} is installed and the remote service is active.`
                      : 'The remote service is active and ready to connect.'}
                  </AlertDescription>
                </Alert>
              )}
            </>
          )}
        </div>

        <DialogFooter className="shrink-0">
          {!server ? (
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Close
            </Button>
          ) : isComplete ? (
            <Button onClick={() => onOpenChange(false)} className="w-full sm:w-auto">
              Done
            </Button>
          ) : isError ? (
            <>
              <Button
                variant="outline"
                onClick={() => {
                  resetState()
                  void startProvisioning()
                }}
              >
                Retry
              </Button>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Close
              </Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button onClick={startProvisioning} disabled={running}>
                {running && <Loader2 className="size-4 animate-spin" />}
                Provision server
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
