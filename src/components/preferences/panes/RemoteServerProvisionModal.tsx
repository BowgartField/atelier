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
import { listen } from '@/lib/transport'
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
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import { useProvisionRemoteServer } from '@/services/remote-servers'
import type {
  RemoteProvisionLogLine,
  RemoteProvisionProgress,
  RemoteServerConfig,
} from '@/types/remote'

interface ProvisionStep {
  stage: string
  label: string
  description: string
}

const PROVISION_STEPS: ProvisionStep[] = [
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
  {
    stage: 'complete',
    label: 'Ready',
    description: 'Remote Jean is available through the SSH tunnel.',
  },
]

interface RemoteServerProvisionModalProps {
  open: boolean
  server: RemoteServerConfig | null
  onOpenChange: (open: boolean) => void
}

function stageIndex(stage?: string | null): number {
  if (!stage) return -1
  return PROVISION_STEPS.findIndex(step => step.stage === stage)
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
      className="min-h-0 flex-1 rounded-xl border bg-muted/10"
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
}: {
  progress: RemoteProvisionProgress | null
}) {
  const activeIndex = stageIndex(progress?.stage)
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {PROVISION_STEPS.map((step, index) => {
        const completed = activeIndex > index || progress?.stage === 'complete'
        const active = activeIndex === index && progress?.stage !== 'complete'
        return (
          <div
            key={step.stage}
            className={cn(
              'rounded-xl border p-3 transition-colors',
              completed && 'border-emerald-500/25 bg-emerald-500/5',
              active && 'border-sky-500/25 bg-sky-500/5'
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
                  !completed &&
                    !active &&
                    'border-border bg-background text-muted-foreground'
                )}
              >
                {completed ? (
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
  const [progress, setProgress] = useState<RemoteProvisionProgress | null>(null)
  const [logs, setLogs] = useState<RemoteProvisionLogLine[]>([])
  const [error, setError] = useState<string | null>(null)
  const [completedVersion, setCompletedVersion] = useState<string | null>(null)
  const [running, setRunning] = useState(false)
  const initializedRef = useRef(false)
  const logsViewportRef = useRef<HTMLDivElement | null>(null)

  const serverId = server?.id ?? null

  const resetState = useCallback(() => {
    setProgress(null)
    setLogs([])
    setError(null)
    setCompletedVersion(null)
    setRunning(false)
    initializedRef.current = false
  }, [])

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

  const currentStep = useMemo(() => {
    if (progress?.stage === 'complete') return PROVISION_STEPS[PROVISION_STEPS.length - 1]
    return PROVISION_STEPS.find(step => step.stage === progress?.stage) ?? null
  }, [progress?.stage])

  const startProvisioning = useCallback(async () => {
    if (!server || running || initializedRef.current) return
    initializedRef.current = true
    setRunning(true)
    setError(null)
    setLogs([])
    setCompletedVersion(null)

    try {
      const result = await provisionServer.mutateAsync(server.id)
      setCompletedVersion(result.version)
      setProgress({
        server_id: server.id,
        stage: 'complete',
        message: `Jean ${result.version} is running`,
        percent: 100,
      })
      setRunning(false)
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause)
      setError(message)
      setRunning(false)
      toast.error(`Provisioning failed: ${message}`)
    }
  }, [provisionServer, running, server])

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

              <StepRail progress={progress} />

              <div className="min-h-[220px] flex-1">
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

        <DialogFooter className="shrink-0 gap-2 sm:gap-0">
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
