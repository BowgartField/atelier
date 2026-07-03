import { useMemo, useState } from 'react'
import {
  ArrowLeft,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Clock,
  ExternalLink,
  Loader2,
  MinusCircle,
  XCircle,
} from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { ModalCloseButton } from '@/components/ui/modal-close-button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useWorkflowJobLogs, useWorkflowRun } from '@/services/github'
import { openExternal } from '@/lib/platform'
import type {
  WorkflowJobDefinition,
  WorkflowJobLogLine,
  WorkflowRun,
  WorkflowRunJob,
  WorkflowRunStep,
} from '@/types/github'

type WorkflowStateKind =
  | 'running'
  | 'success'
  | 'failure'
  | 'skipped'
  | 'pending'

const NODE_WIDTH = 280
const NODE_HEIGHT = 72
const COLUMN_GAP = 112
const ROW_GAP = 28
const GRAPH_PADDING = 32

function getStateKind(
  status: string,
  conclusion?: string | null
): WorkflowStateKind {
  if (status === 'in_progress' || status === 'queued') return 'running'
  if (conclusion === 'success') return 'success'
  if (conclusion === 'failure' || conclusion === 'startup_failure')
    return 'failure'
  if (conclusion === 'cancelled' || conclusion === 'skipped') return 'skipped'
  return 'pending'
}

function StateIcon({
  status,
  conclusion,
  className = 'h-4 w-4',
}: {
  status: string
  conclusion?: string | null
  className?: string
}) {
  const kind = getStateKind(status, conclusion)
  if (kind === 'success') {
    return <CheckCircle2 className={`${className} shrink-0 text-green-500`} />
  }
  if (kind === 'failure') {
    return <XCircle className={`${className} shrink-0 text-red-500`} />
  }
  if (kind === 'running') {
    return (
      <Loader2
        className={`${className} shrink-0 animate-spin text-yellow-500`}
      />
    )
  }
  if (kind === 'skipped') {
    return (
      <MinusCircle className={`${className} shrink-0 text-muted-foreground`} />
    )
  }
  return <Clock className={`${className} shrink-0 text-muted-foreground`} />
}

function stateLabel(status: string, conclusion?: string | null) {
  const kind = getStateKind(status, conclusion)
  if (kind === 'success') return 'Success'
  if (kind === 'failure') return 'Failed'
  if (kind === 'running') return 'Running'
  if (kind === 'skipped') return 'Skipped'
  return 'Queued'
}

function formatDuration(
  startedAt?: string | null,
  completedAt?: string | null
) {
  if (!startedAt) return null
  const start = new Date(startedAt).getTime()
  const end = completedAt ? new Date(completedAt).getTime() : Date.now()
  const seconds = Math.max(0, Math.round((end - start) / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const remainingSeconds = seconds % 60
  return `${minutes}m ${remainingSeconds}s`
}

function timeAgo(dateString: string) {
  const seconds = Math.floor(
    (Date.now() - new Date(dateString).getTime()) / 1000
  )
  if (seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  return `${Math.floor(hours / 24)}d ago`
}

function logsForStep(
  logs: WorkflowJobLogLine[],
  step: WorkflowRunStep
): WorkflowJobLogLine[] {
  const exactMatches = logs.filter(log => log.stepName === step.name)
  if (exactMatches.length > 0) return exactMatches
  if (!step.startedAt) return []

  const start = new Date(step.startedAt).getTime()
  const end = step.completedAt
    ? new Date(step.completedAt).getTime() + 1000
    : Number.POSITIVE_INFINITY
  return logs.filter(log => {
    if (!log.timestamp) return false
    const timestamp = new Date(log.timestamp).getTime()
    return timestamp >= start && timestamp <= end
  })
}

function logTimestamp(timestamp: string | null) {
  if (!timestamp) return ''
  return timestamp.match(/T(\d{2}:\d{2}:\d{2})/)?.[1] ?? timestamp
}

function parseLogMessage(message: string) {
  const annotation = message.match(
    /^##\[(error|warning|notice|debug)(?: [^\]]*)?\](.*)$/i
  )
  if (annotation) {
    return {
      level: annotation[1]?.toLowerCase() ?? 'default',
      message: annotation[2] ?? '',
    }
  }

  return {
    level: 'default',
    message: message.replace(/^##\[group\]/, ''),
  }
}

function logLineClass(level: string) {
  if (level === 'error') return 'bg-red-500/10 hover:bg-red-500/15'
  if (level === 'warning') return 'bg-yellow-500/10 hover:bg-yellow-500/15'
  if (level === 'notice') return 'bg-blue-500/10 hover:bg-blue-500/15'
  return 'hover:bg-white/5'
}

function logMessageClass(level: string) {
  if (level === 'error') return 'font-semibold text-red-400'
  if (level === 'warning') return 'font-semibold text-yellow-300'
  if (level === 'notice') return 'text-blue-300'
  if (level === 'debug') return 'text-[#8b949e]'
  return ''
}

function isVisibleLogLine(line: WorkflowJobLogLine) {
  return line.message.trim() !== '##[endgroup]'
}

function matchesDefinition(jobName: string, definition: WorkflowJobDefinition) {
  return [definition.name, definition.id].some(
    candidate =>
      jobName === candidate ||
      jobName.startsWith(`${candidate} (`) ||
      jobName.startsWith(`${candidate} /`)
  )
}

interface GraphNode {
  job: WorkflowRunJob
  dependencies: number[]
  level: number
  x: number
  y: number
}

function buildGraph(
  jobs: WorkflowRunJob[],
  definitions: WorkflowJobDefinition[]
): GraphNode[] {
  const definitionByJob = new Map<number, WorkflowJobDefinition>()
  const jobsByDefinition = new Map<string, WorkflowRunJob[]>()

  for (const job of jobs) {
    const definition = definitions.find(item =>
      matchesDefinition(job.name, item)
    )
    if (!definition) continue
    definitionByJob.set(job.databaseId, definition)
    jobsByDefinition.set(definition.id, [
      ...(jobsByDefinition.get(definition.id) ?? []),
      job,
    ])
  }

  const dependenciesByJob = new Map<number, number[]>()
  for (const job of jobs) {
    const definition = definitionByJob.get(job.databaseId)
    const dependencies =
      definition?.needs.flatMap(
        dependency =>
          jobsByDefinition.get(dependency)?.map(item => item.databaseId) ?? []
      ) ?? []
    dependenciesByJob.set(job.databaseId, dependencies)
  }

  const levels = new Map<number, number>()
  const resolveLevel = (
    jobId: number,
    visiting = new Set<number>()
  ): number => {
    const existing = levels.get(jobId)
    if (existing != null) return existing
    if (visiting.has(jobId)) return 0

    const nextVisiting = new Set(visiting).add(jobId)
    const dependencies = dependenciesByJob.get(jobId) ?? []
    const level =
      dependencies.length === 0
        ? 0
        : Math.max(
            ...dependencies.map(dependency =>
              resolveLevel(dependency, nextVisiting)
            )
          ) + 1
    levels.set(jobId, level)
    return level
  }

  jobs.forEach(job => resolveLevel(job.databaseId))
  const jobsByLevel = new Map<number, WorkflowRunJob[]>()
  for (const job of jobs) {
    const level = levels.get(job.databaseId) ?? 0
    jobsByLevel.set(level, [...(jobsByLevel.get(level) ?? []), job])
  }

  return jobs.map(job => {
    const level = levels.get(job.databaseId) ?? 0
    const row = jobsByLevel
      .get(level)
      ?.findIndex(item => item.databaseId === job.databaseId)
    return {
      job,
      dependencies: dependenciesByJob.get(job.databaseId) ?? [],
      level,
      x: GRAPH_PADDING + level * (NODE_WIDTH + COLUMN_GAP),
      y: GRAPH_PADDING + Math.max(0, row ?? 0) * (NODE_HEIGHT + ROW_GAP),
    }
  })
}

function WorkflowGraph({
  jobs,
  definitions,
  selectedJobId,
  onSelectJob,
}: {
  jobs: WorkflowRunJob[]
  definitions: WorkflowJobDefinition[]
  selectedJobId: number | null
  onSelectJob: (jobId: number) => void
}) {
  const nodes = useMemo(
    () => buildGraph(jobs, definitions),
    [jobs, definitions]
  )
  const nodeById = useMemo(
    () => new Map(nodes.map(node => [node.job.databaseId, node])),
    [nodes]
  )
  const maxLevel = Math.max(0, ...nodes.map(node => node.level))
  const maxRows = Math.max(
    1,
    ...Array.from(
      { length: maxLevel + 1 },
      (_, level) => nodes.filter(node => node.level === level).length
    )
  )
  const width =
    GRAPH_PADDING * 2 + (maxLevel + 1) * NODE_WIDTH + maxLevel * COLUMN_GAP
  const height =
    GRAPH_PADDING * 2 + maxRows * NODE_HEIGHT + (maxRows - 1) * ROW_GAP

  return (
    <div className="overflow-auto rounded-lg border border-border bg-background">
      <div
        className="relative min-h-[250px] min-w-full"
        style={{ width, height: Math.max(250, height) }}
      >
        <svg
          aria-hidden="true"
          className="pointer-events-none absolute inset-0"
          width={width}
          height={Math.max(250, height)}
        >
          {nodes.flatMap(node =>
            node.dependencies.map(dependencyId => {
              const dependency = nodeById.get(dependencyId)
              if (!dependency) return null
              const startX = dependency.x + NODE_WIDTH
              const startY = dependency.y + NODE_HEIGHT / 2
              const endX = node.x
              const endY = node.y + NODE_HEIGHT / 2
              const curve = Math.max(32, (endX - startX) / 2)
              return (
                <path
                  key={`${dependencyId}-${node.job.databaseId}`}
                  d={`M ${startX} ${startY} C ${startX + curve} ${startY}, ${endX - curve} ${endY}, ${endX} ${endY}`}
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  className="text-border"
                />
              )
            })
          )}
        </svg>

        {nodes.map(node => {
          const selected = node.job.databaseId === selectedJobId
          return (
            <button
              key={node.job.databaseId}
              type="button"
              aria-label={`View ${node.job.name} job details`}
              onClick={() => onSelectJob(node.job.databaseId)}
              className={`absolute flex items-center gap-3 rounded-lg border bg-muted/50 px-4 text-left shadow-sm transition-colors hover:border-muted-foreground/50 hover:bg-muted ${
                selected
                  ? 'border-blue-500 ring-2 ring-blue-500/20'
                  : 'border-border'
              }`}
              style={{
                left: node.x,
                top: node.y,
                width: NODE_WIDTH,
                height: NODE_HEIGHT,
              }}
            >
              {node.dependencies.length > 0 && (
                <span className="absolute -left-2 h-4 w-4 rounded-full border-2 border-background bg-muted-foreground" />
              )}
              <StateIcon
                status={node.job.status}
                conclusion={node.job.conclusion}
                className="h-5 w-5"
              />
              <span className="min-w-0 flex-1 truncate text-sm font-semibold">
                {node.job.name}
              </span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {formatDuration(node.job.startedAt, node.job.completedAt)}
              </span>
              {nodes.some(item =>
                item.dependencies.includes(node.job.databaseId)
              ) && (
                <span className="absolute -right-2 h-4 w-4 rounded-full border-2 border-background bg-muted-foreground" />
              )}
            </button>
          )
        })}
      </div>
    </div>
  )
}

function StepRow({
  step,
  expanded,
  logs,
  isLoading,
  error,
  onToggle,
}: {
  step: WorkflowRunStep
  expanded: boolean
  logs: WorkflowJobLogLine[]
  isLoading: boolean
  error: unknown
  onToggle: () => void
}) {
  const visibleLogs = logs.filter(isVisibleLogLine)

  return (
    <div className="border-b border-border/70 last:border-b-0">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={onToggle}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/30"
      >
        {expanded ? (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <StateIcon status={step.status} conclusion={step.conclusion} />
        <span className="min-w-0 flex-1 truncate text-sm">{step.name}</span>
        <span className="text-xs text-muted-foreground">
          {formatDuration(step.startedAt, step.completedAt)}
        </span>
      </button>

      {expanded && (
        <div className="border-t border-border/70 bg-[#0d1117] text-[#e6edf3]">
          {isLoading ? (
            <div className="flex items-center gap-2 px-4 py-6 text-xs text-[#8b949e]">
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading logs…
            </div>
          ) : error ? (
            <p className="px-4 py-6 text-xs text-red-400">
              Failed to load logs: {String(error)}
            </p>
          ) : visibleLogs.length === 0 ? (
            <p className="px-4 py-6 text-xs text-[#8b949e]">
              No logs were returned for this step.
            </p>
          ) : (
            <div className="max-h-96 overflow-auto py-2 font-mono text-[11px] leading-5">
              {visibleLogs.map((line, index) => {
                const parsed = parseLogMessage(line.message)
                return (
                  <div
                    key={`${line.timestamp ?? 'line'}-${index}`}
                    className={`grid w-full grid-cols-[4rem_minmax(0,1fr)] gap-2 px-3 ${logLineClass(parsed.level)}`}
                  >
                    <span className="select-none text-right text-[#6e7681] tabular-nums">
                      {logTimestamp(line.timestamp)}
                    </span>
                    <span
                      className={`whitespace-pre-wrap break-words ${logMessageClass(parsed.level)}`}
                    >
                      {parsed.message}
                    </span>
                  </div>
                )
              })}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

export function WorkflowRunDetailsModal({
  open,
  projectPath,
  run,
  onOpenChange,
}: {
  open: boolean
  projectPath: string | null
  run: WorkflowRun | null
  onOpenChange: (open: boolean) => void
}) {
  const [selectedJobId, setSelectedJobId] = useState<number | null>(null)
  const [selectedStepNumber, setSelectedStepNumber] = useState<number | null>(
    null
  )
  const { data, isLoading } = useWorkflowRun(
    projectPath,
    run?.databaseId ?? null,
    {
      enabled: open && !!projectPath && !!run,
    }
  )
  const selectedJob =
    data?.jobs.find(job => job.databaseId === selectedJobId) ?? null
  const {
    data: jobLogs = [],
    isLoading: isLoadingJobLogs,
    error: jobLogsError,
  } = useWorkflowJobLogs(
    projectPath,
    run?.databaseId ?? null,
    selectedJob?.databaseId ?? null,
    {
      enabled: open && selectedStepNumber != null,
    }
  )

  const handleSelectJob = (jobId: number | null) => {
    setSelectedJobId(jobId)
    setSelectedStepNumber(null)
  }

  const handleOpenChange = (nextOpen: boolean) => {
    if (!nextOpen) handleSelectJob(null)
    onOpenChange(nextOpen)
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="flex h-[calc(100vh-2rem)] w-[calc(100vw-2rem)] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-[calc(100vw-2rem)]"
      >
        <DialogHeader className="border-b border-border px-5 py-4">
          <div className="flex min-w-0 items-center gap-3">
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8 shrink-0"
              aria-label="Back to workflow runs"
              onClick={() => handleOpenChange(false)}
            >
              <ArrowLeft className="h-4 w-4" />
            </Button>
            {run && (
              <StateIcon
                status={run.status}
                conclusion={run.conclusion}
                className="h-6 w-6"
              />
            )}
            <div className="min-w-0 flex-1">
              <p className="truncate text-xs text-muted-foreground">
                {run?.workflowName ?? 'Workflow run'}
              </p>
              <DialogTitle className="truncate text-base">
                {run?.displayTitle ?? 'Workflow run details'}
              </DialogTitle>
            </div>
            {run && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => openExternal(run.url)}
              >
                <ExternalLink className="mr-2 h-4 w-4" />
                Open on GitHub
              </Button>
            )}
            <ModalCloseButton onClick={() => handleOpenChange(false)} />
          </div>
        </DialogHeader>

        {!run || isLoading ? (
          <div className="flex flex-1 items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : (
          <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[240px_minmax(0,1fr)]">
            <aside className="hidden min-h-0 border-r border-border bg-muted/20 lg:flex lg:flex-col">
              <div className="p-3">
                <button
                  type="button"
                  onClick={() => handleSelectJob(null)}
                  className={`w-full rounded-md px-3 py-2 text-left text-sm font-medium transition-colors ${
                    selectedJobId == null ? 'bg-accent' : 'hover:bg-accent/60'
                  }`}
                >
                  Summary
                </button>
              </div>
              <div className="border-t border-border px-3 py-2 text-xs font-semibold text-muted-foreground">
                All jobs
              </div>
              <ScrollArea className="min-h-0 flex-1">
                <div className="space-y-1 px-3 pb-3">
                  {data?.jobs.map(job => (
                    <button
                      key={job.databaseId}
                      type="button"
                      onClick={() => handleSelectJob(job.databaseId)}
                      className={`flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors ${
                        selectedJobId === job.databaseId
                          ? 'bg-accent'
                          : 'hover:bg-accent/60'
                      }`}
                    >
                      <StateIcon
                        status={job.status}
                        conclusion={job.conclusion}
                      />
                      <span className="min-w-0 truncate">{job.name}</span>
                    </button>
                  ))}
                </div>
              </ScrollArea>
            </aside>

            <ScrollArea className="min-h-0">
              <main className="space-y-5 p-5">
                <section className="grid gap-4 rounded-lg border border-border bg-muted/20 p-4 sm:grid-cols-4">
                  <div className="sm:col-span-2">
                    <p className="text-xs text-muted-foreground">
                      Triggered via
                    </p>
                    <p className="mt-1 text-sm font-medium">
                      {run.event} · {timeAgo(run.createdAt)}
                    </p>
                    <p className="mt-1 truncate text-xs text-muted-foreground">
                      {run.headBranch}
                    </p>
                  </div>
                  <div>
                    <p className="text-xs text-muted-foreground">Status</p>
                    <p className="mt-1 text-sm font-semibold">
                      {stateLabel(run.status, run.conclusion)}
                    </p>
                  </div>
                  <div>
                    <p className="text-xs text-muted-foreground">Jobs</p>
                    <p className="mt-1 text-sm font-semibold">
                      {data?.jobs.length ?? 0}
                    </p>
                  </div>
                </section>

                <section className="rounded-lg border border-border bg-muted/10 p-4">
                  <div className="mb-4">
                    <h2 className="text-base font-semibold">
                      {run.workflowName}
                    </h2>
                    <p className="text-xs text-muted-foreground">
                      on: {run.event}
                    </p>
                  </div>
                  {data?.jobs.length ? (
                    <WorkflowGraph
                      jobs={data.jobs}
                      definitions={data.jobDefinitions ?? []}
                      selectedJobId={selectedJobId}
                      onSelectJob={handleSelectJob}
                    />
                  ) : (
                    <div className="rounded-lg border border-dashed border-border px-4 py-12 text-center text-sm text-muted-foreground">
                      GitHub has not reported any jobs for this run yet.
                    </div>
                  )}
                </section>

                {selectedJob && (
                  <section className="overflow-hidden rounded-lg border border-border">
                    <div className="flex items-center gap-3 border-b border-border bg-muted/30 px-4 py-3">
                      <StateIcon
                        status={selectedJob.status}
                        conclusion={selectedJob.conclusion}
                      />
                      <div className="min-w-0 flex-1">
                        <h2 className="truncate text-sm font-semibold">
                          {selectedJob.name}
                        </h2>
                        <p className="text-xs text-muted-foreground">
                          {stateLabel(
                            selectedJob.status,
                            selectedJob.conclusion
                          )}
                          {formatDuration(
                            selectedJob.startedAt,
                            selectedJob.completedAt
                          )
                            ? ` · ${formatDuration(
                                selectedJob.startedAt,
                                selectedJob.completedAt
                              )}`
                            : ''}
                        </p>
                      </div>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => openExternal(selectedJob.url)}
                      >
                        <ExternalLink className="mr-2 h-4 w-4" />
                        Open job
                      </Button>
                    </div>
                    {selectedJob.steps.length > 0 ? (
                      selectedJob.steps.map(step => (
                        <StepRow
                          key={`${selectedJob.databaseId}-${step.number}`}
                          step={step}
                          expanded={selectedStepNumber === step.number}
                          logs={logsForStep(jobLogs, step)}
                          isLoading={isLoadingJobLogs}
                          error={jobLogsError}
                          onToggle={() =>
                            setSelectedStepNumber(current =>
                              current === step.number ? null : step.number
                            )
                          }
                        />
                      ))
                    ) : (
                      <p className="px-4 py-8 text-center text-sm text-muted-foreground">
                        No steps reported for this job.
                      </p>
                    )}
                  </section>
                )}
              </main>
            </ScrollArea>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
