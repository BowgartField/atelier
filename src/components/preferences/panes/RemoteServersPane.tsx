import { useState, type FormEvent } from 'react'
import {
  Cable,
  CheckCircle2,
  CircleAlert,
  CloudCog,
  KeyRound,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  ServerCog,
  ShieldCheck,
  Trash2,
  Unplug,
} from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { FALLBACK_APP_VERSION } from '@/lib/app-version'
import { isMacOS } from '@/lib/platform'
import { resolveDefaultProvisionVersion } from '@/lib/remote-versions'
import { cn } from '@/lib/utils'
import {
  useClaudeCliAuth,
  useClaudeCliStatus,
} from '@/services/claude-cli'
import {
  useAddRemoteServer,
  useConnectRemoteServer,
  useDisconnectRemoteServer,
  useRemoteJeanVersions,
  useRemoteServers,
  useRemoveRemoteServer,
  useTestRemoteServer,
  useUpdateRemoteServer,
} from '@/services/remote-servers'
import { useUIStore } from '@/store/ui-store'
import type {
  RemoteServerConfig,
  RemoteServerInput,
  RemoteServerStatus,
} from '@/types/remote'
import { SettingsSection } from '../SettingsSection'
import { RemoteServerProvisionModal } from './RemoteServerProvisionModal'

type BusyAction = 'test' | 'connect' | 'disconnect' | 'delete'

interface ServerFormState {
  name: string
  host: string
  port: string
  username: string
  authType: 'ssh_key_path' | 'password'
  keyPath: string
  keyPassphrase: string
  password: string
  remotePort: string
  isDefault: boolean
}

const EMPTY_FORM: ServerFormState = {
  name: '',
  host: '',
  port: '22',
  username: 'root',
  authType: 'ssh_key_path',
  keyPath: '~/.ssh/id_ed25519',
  keyPassphrase: '',
  password: '',
  remotePort: '3456',
  isDefault: false,
}

function RemoteClaudeAuthStatus({
  server,
  connected,
}: {
  server: RemoteServerConfig
  connected: boolean
}) {
  const openCliLoginModal = useUIStore(state => state.openCliLoginModal)
  const cliStatus = useClaudeCliStatus({
    enabled: connected,
    serverId: server.id,
  })
  const cliAuth = useClaudeCliAuth({
    enabled: connected && cliStatus.data?.installed === true,
    serverId: server.id,
  })

  const handleLogin = () => {
    const status = cliStatus.data
    if (!status?.path) return
    const args = status.supports_auth_command ? ['auth', 'login'] : ['login']
    openCliLoginModal('claude', status.path, args, 'login', server.id)
  }

  let statusLabel = 'Connect to check'
  if (connected && cliStatus.isLoading) {
    statusLabel = 'Checking installation…'
  } else if (connected && !cliStatus.data?.installed) {
    statusLabel = 'Not installed'
  } else if (connected && cliAuth.isLoading) {
    statusLabel = 'Checking login…'
  } else if (connected && cliAuth.data?.authenticated) {
    statusLabel = 'Logged in'
  } else if (connected && cliStatus.data?.installed) {
    statusLabel = 'Login required'
  }

  return (
    <div className="flex items-center gap-3 rounded-lg border bg-muted/15 px-3 py-2.5">
      <KeyRound className="size-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <p className="text-xs font-medium">Claude CLI</p>
        <p className="text-xs text-muted-foreground">{statusLabel}</p>
      </div>
      {connected &&
        cliStatus.data?.installed &&
        !cliAuth.isLoading &&
        !cliAuth.data?.authenticated && (
          <Button variant="outline" size="sm" onClick={handleLogin}>
            Login
          </Button>
        )}
    </div>
  )
}

function formFromServer(server: RemoteServerConfig): ServerFormState {
  return {
    name: server.name,
    host: server.host,
    port: String(server.port),
    username: server.username,
    authType: server.auth.type,
    keyPath:
      server.auth.type === 'ssh_key_path'
        ? server.auth.path
        : EMPTY_FORM.keyPath,
    keyPassphrase:
      server.auth.type === 'ssh_key_path' ? (server.auth.passphrase ?? '') : '',
    password: server.auth.type === 'password' ? server.auth.password : '',
    remotePort: String(server.remote_port),
    isDefault: server.default,
  }
}

function parsePort(value: string, label: string): number {
  const port = Number(value)
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`${label} must be between 1 and 65535`)
  }
  return port
}

function toServerInput(form: ServerFormState): RemoteServerInput {
  const name = form.name.trim()
  const host = form.host.trim()
  const username = form.username.trim()
  if (!name || !host || !username) {
    throw new Error('Name, host, and username are required')
  }

  const auth =
    form.authType === 'ssh_key_path'
      ? {
          type: 'ssh_key_path' as const,
          path: form.keyPath.trim(),
          passphrase: form.keyPassphrase || undefined,
        }
      : { type: 'password' as const, password: form.password }
  if (
    (auth.type === 'ssh_key_path' && !auth.path) ||
    (auth.type === 'password' && !auth.password)
  ) {
    throw new Error(
      auth.type === 'ssh_key_path'
        ? 'SSH key path is required'
        : 'SSH password is required'
    )
  }

  return {
    name,
    host,
    port: parsePort(form.port, 'SSH port'),
    username,
    auth,
    default: form.isDefault,
    remote_port: parsePort(form.remotePort, 'Remote Jean port'),
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

const STATUS_META: Record<
  RemoteServerStatus,
  { label: string; dot: string; badge: string }
> = {
  disconnected: {
    label: 'Disconnected',
    dot: 'bg-muted-foreground/50',
    badge: 'border-border bg-muted/40 text-muted-foreground',
  },
  connecting: {
    label: 'Connecting',
    dot: 'bg-sky-500 animate-pulse',
    badge: 'border-sky-500/25 bg-sky-500/10 text-sky-600 dark:text-sky-400',
  },
  connected: {
    label: 'Connected',
    dot: 'bg-emerald-500',
    badge:
      'border-emerald-500/25 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400',
  },
  provisioning: {
    label: 'Provisioning',
    dot: 'bg-amber-500 animate-pulse',
    badge:
      'border-amber-500/25 bg-amber-500/10 text-amber-600 dark:text-amber-400',
  },
  error: {
    label: 'Error',
    dot: 'bg-destructive',
    badge:
      'border-destructive/25 bg-destructive/10 text-destructive dark:text-red-400',
  },
}

function StatusBadge({ status }: { status: RemoteServerStatus }) {
  const meta = STATUS_META[status]
  return (
    <Badge variant="outline" className={cn('gap-1.5', meta.badge)}>
      <span className={cn('size-1.5 rounded-full', meta.dot)} />
      {meta.label}
    </Badge>
  )
}

interface RemoteServerFormDialogProps {
  open: boolean
  server: RemoteServerConfig | null
  submitting: boolean
  onOpenChange: (open: boolean) => void
  onSubmit: (input: RemoteServerInput) => Promise<void>
}

function RemoteServerFormDialog({
  open,
  server,
  submitting,
  onOpenChange,
  onSubmit,
}: RemoteServerFormDialogProps) {
  const [form, setForm] = useState<ServerFormState>(() =>
    server ? formFromServer(server) : EMPTY_FORM
  )
  const [validationError, setValidationError] = useState<string | null>(null)

  const handleOpenChange = (nextOpen: boolean) => {
    onOpenChange(nextOpen)
  }

  const update = <K extends keyof ServerFormState>(
    key: K,
    value: ServerFormState[K]
  ) => setForm(current => ({ ...current, [key]: value }))

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault()
    try {
      setValidationError(null)
      await onSubmit(toServerInput(form))
    } catch (error) {
      setValidationError(errorMessage(error))
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {server ? 'Edit remote server' : 'Add remote server'}
          </DialogTitle>
          <DialogDescription>
            Jean connects through your system SSH client and keeps the remote
            backend bound to loopback.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-5">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2 sm:col-span-2">
              <Label htmlFor="remote-name">Display name</Label>
              <Input
                id="remote-name"
                autoFocus
                value={form.name}
                onChange={event => update('name', event.target.value)}
                placeholder="Production box"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="remote-host">Host or IP address</Label>
              <Input
                id="remote-host"
                value={form.host}
                onChange={event => update('host', event.target.value)}
                placeholder="203.0.113.10"
                autoCapitalize="none"
                spellCheck={false}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="remote-ssh-port">SSH port</Label>
              <Input
                id="remote-ssh-port"
                type="number"
                min={1}
                max={65535}
                value={form.port}
                onChange={event => update('port', event.target.value)}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="remote-username">SSH username</Label>
              <Input
                id="remote-username"
                value={form.username}
                onChange={event => update('username', event.target.value)}
                placeholder="root"
                autoCapitalize="none"
                spellCheck={false}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="remote-jean-port">Remote Jean port</Label>
              <Input
                id="remote-jean-port"
                type="number"
                min={1}
                max={65535}
                value={form.remotePort}
                onChange={event => update('remotePort', event.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                Bound to 127.0.0.1 on the server.
              </p>
            </div>
          </div>

          <div className="rounded-lg border bg-muted/20 p-4">
            <div className="mb-4 flex items-center gap-2">
              <KeyRound className="size-4 text-muted-foreground" />
              <span className="text-sm font-medium">Authentication</span>
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2 sm:col-span-2">
                <Label htmlFor="remote-auth-type">Method</Label>
                <NativeSelect
                  id="remote-auth-type"
                  className="w-full"
                  value={form.authType}
                  onChange={event =>
                    update(
                      'authType',
                      event.target.value as ServerFormState['authType']
                    )
                  }
                >
                  <NativeSelectOption value="ssh_key_path">
                    SSH key
                  </NativeSelectOption>
                  <NativeSelectOption value="password">
                    Password
                  </NativeSelectOption>
                </NativeSelect>
              </div>
              {form.authType === 'ssh_key_path' ? (
                <>
                  <div className="space-y-2">
                    <Label htmlFor="remote-key-path">Private key path</Label>
                    <Input
                      id="remote-key-path"
                      value={form.keyPath}
                      onChange={event => update('keyPath', event.target.value)}
                      placeholder="~/.ssh/id_ed25519"
                      autoCapitalize="none"
                      spellCheck={false}
                    />
                  </div>
                  {isMacOS ? (
                    <div className="space-y-2">
                      <Label htmlFor="remote-key-passphrase">
                        Key passphrase{' '}
                        <span className="font-normal text-muted-foreground">
                          (optional)
                        </span>
                      </Label>
                      <Input
                        id="remote-key-passphrase"
                        type="password"
                        value={form.keyPassphrase}
                        onChange={event =>
                          update('keyPassphrase', event.target.value)
                        }
                        autoComplete="new-password"
                        placeholder={
                          server
                            ? 'Leave blank to keep stored value'
                            : 'Required for encrypted keys'
                        }
                      />
                      <p className="text-xs text-muted-foreground">
                        Stored securely in macOS Keychain.
                      </p>
                    </div>
                  ) : (
                    <div className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
                      Encrypted-key passphrases are currently supported on
                      macOS.
                    </div>
                  )}
                </>
              ) : (
                <div className="space-y-2 sm:col-span-2">
                  <Label htmlFor="remote-password">Password</Label>
                  <Input
                    id="remote-password"
                    type="password"
                    value={form.password}
                    onChange={event => update('password', event.target.value)}
                    autoComplete="new-password"
                  />
                </div>
              )}
            </div>
          </div>

          {form.authType === 'password' && (
            <Alert>
              <CircleAlert />
              <AlertTitle>Password stored locally</AlertTitle>
              <AlertDescription>
                SSH passwords are currently stored in Jean&apos;s local
                preferences file. Prefer an SSH key with a Keychain-backed
                passphrase.
              </AlertDescription>
            </Alert>
          )}

          <div className="flex items-center justify-between gap-4 rounded-lg border p-4">
            <div>
              <Label htmlFor="remote-default">Default remote server</Label>
              <p className="mt-1 text-xs text-muted-foreground">
                Used as the initial target when remote project creation lands.
              </p>
            </div>
            <Switch
              id="remote-default"
              checked={form.isDefault}
              onCheckedChange={checked => update('isDefault', checked)}
            />
          </div>

          {validationError && (
            <Alert variant="destructive">
              <CircleAlert />
              <AlertTitle>Could not save server</AlertTitle>
              <AlertDescription>{validationError}</AlertDescription>
            </Alert>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={submitting}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={submitting}>
              {submitting && <Loader2 className="animate-spin" />}
              {server ? 'Save changes' : 'Add server'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

export function RemoteServersPane() {
  const serversQuery = useRemoteServers()
  const { data: availableVersions } = useRemoteJeanVersions(true)
  const addServer = useAddRemoteServer()
  const updateServer = useUpdateRemoteServer()
  const removeServer = useRemoveRemoteServer()
  const testServer = useTestRemoteServer()
  const connectServer = useConnectRemoteServer()
  const disconnectServer = useDisconnectRemoteServer()
  const [formOpen, setFormOpen] = useState(false)
  const [editingServer, setEditingServer] = useState<RemoteServerConfig | null>(
    null
  )
  const [deleteTarget, setDeleteTarget] = useState<RemoteServerConfig | null>(
    null
  )
  const [provisionTarget, setProvisionTarget] =
    useState<RemoteServerConfig | null>(null)
  const [busy, setBusy] = useState<{
    serverId: string
    action: BusyAction
  } | null>(null)

  const servers = serversQuery.data ?? []
  const isSubmitting = addServer.isPending || updateServer.isPending
  const desktopVersionPublished = (availableVersions ?? []).some(
    entry => entry.version === FALLBACK_APP_VERSION
  )
  const updateTarget = resolveDefaultProvisionVersion(availableVersions)

  const openAdd = () => {
    setEditingServer(null)
    setFormOpen(true)
  }

  const openEdit = (server: RemoteServerConfig) => {
    setEditingServer(server)
    setFormOpen(true)
  }

  const handleSave = async (config: RemoteServerInput) => {
    if (editingServer) {
      await updateServer.mutateAsync({
        serverId: editingServer.id,
        config,
      })
      toast.success('Remote server updated')
    } else {
      await addServer.mutateAsync(config)
      toast.success('Remote server added')
    }
    setFormOpen(false)
  }

  const runAction = async (
    server: RemoteServerConfig,
    action: BusyAction,
    operation: () => Promise<void>
  ) => {
    setBusy({ serverId: server.id, action })
    try {
      await operation()
    } finally {
      setBusy(null)
    }
  }

  const handleTest = (server: RemoteServerConfig) =>
    runAction(server, 'test', async () => {
      const toastId = toast.loading(`Testing SSH to ${server.name}…`)
      try {
        const result = await testServer.mutateAsync(server.id)
        if (!result.success) throw new Error(result.message)
        toast.success(
          result.hostname
            ? `Connected to ${result.hostname}`
            : 'SSH connection successful',
          { id: toastId }
        )
      } catch (error) {
        toast.error(`SSH test failed: ${errorMessage(error)}`, { id: toastId })
      }
    })

  const handleConnect = (server: RemoteServerConfig) =>
    runAction(server, 'connect', async () => {
      const toastId = toast.loading(`Opening tunnel to ${server.name}…`)
      try {
        const connection = await connectServer.mutateAsync(server.id)
        toast.success(`Tunnel ready on 127.0.0.1:${connection.local_port}`, {
          id: toastId,
        })
      } catch (error) {
        toast.error(`Connection failed: ${errorMessage(error)}`, {
          id: toastId,
        })
      }
    })

  const handleDisconnect = (server: RemoteServerConfig) =>
    runAction(server, 'disconnect', async () => {
      try {
        await disconnectServer.mutateAsync(server.id)
        toast.success(`Disconnected from ${server.name}`)
      } catch (error) {
        toast.error(`Disconnect failed: ${errorMessage(error)}`)
      }
    })

  const handleDelete = async () => {
    if (!deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    await runAction(target, 'delete', async () => {
      try {
        await removeServer.mutateAsync(target.id)
        toast.success('Remote server removed')
      } catch (error) {
        toast.error(`Could not remove server: ${errorMessage(error)}`)
      }
    })
  }

  const isBusy = (server: RemoteServerConfig, action?: BusyAction) =>
    busy?.serverId === server.id && (!action || busy.action === action)

  return (
    <div className="space-y-6">
      <SettingsSection
        title={
          <span className="inline-flex items-center gap-2">
            <CloudCog className="size-5 text-muted-foreground" />
            Remote Servers
          </span>
        }
        description="Provision and connect to headless Jean backends over encrypted SSH tunnels."
        actions={
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() => serversQuery.refetch()}
              disabled={serversQuery.isFetching}
              aria-label="Refresh remote servers"
            >
              <RefreshCw
                className={cn(serversQuery.isFetching && 'animate-spin')}
              />
              Refresh
            </Button>
            <Button size="sm" onClick={openAdd}>
              <Plus />
              Add server
            </Button>
          </>
        }
        anchorId="pref-remote-servers-section-list"
      >
        <Alert className="border-sky-500/20 bg-sky-500/5">
          <ShieldCheck className="text-sky-500" />
          <AlertTitle>Private by default</AlertTitle>
          <AlertDescription>
            Remote Jean binds to 127.0.0.1. Only the local SSH forward can reach
            its authenticated HTTP and WebSocket API.
          </AlertDescription>
        </Alert>

        {serversQuery.isLoading ? (
          <div className="grid gap-3 lg:grid-cols-2">
            {[0, 1].map(index => (
              <div
                key={index}
                className="h-48 animate-pulse rounded-xl border bg-muted/30"
              />
            ))}
          </div>
        ) : serversQuery.isError ? (
          <Alert variant="destructive">
            <CircleAlert />
            <AlertTitle>Could not load remote servers</AlertTitle>
            <AlertDescription>
              {errorMessage(serversQuery.error)}
            </AlertDescription>
          </Alert>
        ) : servers.length === 0 ? (
          <Empty className="min-h-72 border bg-muted/10">
            <EmptyHeader>
              <EmptyMedia
                variant="icon"
                className="size-12 rounded-xl bg-sky-500/10 text-sky-500"
              >
                <ServerCog />
              </EmptyMedia>
              <EmptyTitle>No remote servers yet</EmptyTitle>
              <EmptyDescription>
                Add a Linux host, verify SSH access, then let Jean install and
                start its headless backend.
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button onClick={openAdd}>
                <Plus />
                Add your first server
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <div className="grid gap-3 lg:grid-cols-2">
            {servers.map(server => {
              const status = server.status ?? 'disconnected'
              const connected = status === 'connected'
              const globallyBusy = isBusy(server)
              const versionMismatch =
                !!server.installed_version &&
                server.installed_version !== FALLBACK_APP_VERSION
              return (
                <Card
                  key={server.id}
                  data-testid={`remote-server-${server.id}`}
                  className={cn(
                    'gap-4 overflow-hidden py-0 transition-colors',
                    connected && 'border-emerald-500/30'
                  )}
                >
                  <CardHeader className="border-b bg-muted/15 px-5 py-4">
                    <CardTitle className="flex min-w-0 items-center gap-2">
                      <span
                        className={cn(
                          'grid size-8 shrink-0 place-items-center rounded-lg border bg-background',
                          connected &&
                            'border-emerald-500/30 bg-emerald-500/10 text-emerald-500'
                        )}
                      >
                        <ServerCog className="size-4" />
                      </span>
                      <span className="min-w-0 truncate">{server.name}</span>
                    </CardTitle>
                    <CardDescription className="truncate font-mono text-xs">
                      {server.username}@{server.host}:{server.port}
                    </CardDescription>
                    <CardAction className="flex items-center gap-2">
                      {server.default && <Badge variant="muted">Default</Badge>}
                      <StatusBadge status={status} />
                    </CardAction>
                  </CardHeader>

                  <CardContent className="space-y-4 px-5 pb-5">
                    <div className="grid grid-cols-2 gap-3 text-sm">
                      <div className="rounded-lg border bg-muted/15 p-3">
                        <p className="text-xs text-muted-foreground">
                          Jean endpoint
                        </p>
                        <p className="mt-1 font-mono text-xs">
                          127.0.0.1:{server.remote_port}
                        </p>
                      </div>
                      <div className="rounded-lg border bg-muted/15 p-3">
                        <p className="text-xs text-muted-foreground">
                          Installed version
                        </p>
                        <p
                          className={cn(
                            'mt-1 font-mono text-xs',
                            versionMismatch && 'text-amber-600 dark:text-amber-400'
                          )}
                        >
                          {server.installed_version ?? 'Not provisioned'}
                        </p>
                      </div>
                    </div>

                    <RemoteClaudeAuthStatus
                      server={server}
                      connected={connected}
                    />

                    {versionMismatch ? (
                      <div className="flex items-start gap-2 rounded-lg border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
                        <CircleAlert className="mt-0.5 size-3.5 shrink-0" />
                        <span>
                          Remote Jean {server.installed_version} does not
                          match this app ({FALLBACK_APP_VERSION}). Update the
                          remote to connect.
                          {!desktopVersionPublished && (
                            <>
                              {' '}
                              This build has no published release yet, so
                              updating installs the latest release (
                              {updateTarget}) instead. Packaged release builds
                              still require an exact version match to connect;
                              dev builds skip that check.
                            </>
                          )}
                        </span>
                      </div>
                    ) : (
                      status === 'error' && (
                        <div className="flex items-start gap-2 rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                          <CircleAlert className="mt-0.5 size-3.5 shrink-0" />
                          Check SSH access or retry provisioning to restore this
                          server.
                        </div>
                      )
                    )}

                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => handleTest(server)}
                        disabled={globallyBusy}
                      >
                        {isBusy(server, 'test') ? (
                          <Loader2 className="animate-spin" />
                        ) : (
                          <CheckCircle2 />
                        )}
                        Test SSH
                      </Button>
                      <Button
                        variant={versionMismatch ? 'default' : 'outline'}
                        size="sm"
                        onClick={() => setProvisionTarget(server)}
                        disabled={globallyBusy || connected}
                      >
                        <ShieldCheck />
                        {versionMismatch
                          ? `Update to ${updateTarget}`
                          : server.installed_version
                            ? 'Reprovision'
                            : 'Provision'}
                      </Button>
                      {connected ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => handleDisconnect(server)}
                          disabled={globallyBusy}
                        >
                          {isBusy(server, 'disconnect') ? (
                            <Loader2 className="animate-spin" />
                          ) : (
                            <Unplug />
                          )}
                          Disconnect
                        </Button>
                      ) : (
                        <Button
                          size="sm"
                          onClick={() => handleConnect(server)}
                          disabled={globallyBusy || !server.http_token}
                          title={
                            server.http_token
                              ? undefined
                              : 'Provision Jean before connecting'
                          }
                        >
                          {isBusy(server, 'connect') ? (
                            <Loader2 className="animate-spin" />
                          ) : (
                            <Cable />
                          )}
                          Connect
                        </Button>
                      )}
                      <div className="ml-auto flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => openEdit(server)}
                          disabled={globallyBusy || connected}
                          aria-label={`Edit ${server.name}`}
                        >
                          <Pencil />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => setDeleteTarget(server)}
                          disabled={globallyBusy}
                          aria-label={`Remove ${server.name}`}
                          className="text-muted-foreground hover:text-destructive"
                        >
                          <Trash2 />
                        </Button>
                      </div>
                    </div>
                  </CardContent>
                </Card>
              )
            })}
          </div>
        )}
      </SettingsSection>

      <Alert className="bg-muted/15">
        <CloudCog />
        <AlertTitle>Project routing comes next</AlertTitle>
        <AlertDescription>
          Connecting opens and verifies the SSH tunnel. Assigning projects and
          sessions to that backend is part of the transport-routing phase.
        </AlertDescription>
      </Alert>

      {formOpen && (
        <RemoteServerFormDialog
          open
          server={editingServer}
          submitting={isSubmitting}
          onOpenChange={setFormOpen}
          onSubmit={handleSave}
        />
      )}

      <RemoteServerProvisionModal
        open={provisionTarget != null}
        server={provisionTarget}
        onOpenChange={open => !open && setProvisionTarget(null)}
      />

      <AlertDialog
        open={deleteTarget != null}
        onOpenChange={open => !open && setDeleteTarget(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove remote server?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes {deleteTarget?.name} from Jean and closes its active
              tunnel. It does not uninstall Jean or delete data on the server.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90"
              onClick={handleDelete}
            >
              Remove server
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
