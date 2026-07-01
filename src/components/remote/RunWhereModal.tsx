import { useCallback } from 'react'
import { Laptop, Server } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { useRemoteServers } from '@/services/remote-servers'
import type { RemoteServerConfig } from '@/types/remote'

interface RunWhereModalProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Called with null for local, or serverId for remote */
  onSelect: (serverId: string | null) => void
  /** Project name for display */
  projectName?: string
  /** Server IDs this project has been cloned to (only show cloned servers) */
  clonedServerIds?: string[]
}

export function RunWhereModal({
  open,
  onOpenChange,
  onSelect,
  projectName,
  clonedServerIds,
}: RunWhereModalProps) {
  const { data: remoteServers = [] } = useRemoteServers()

  const isReady = (s: RemoteServerConfig) =>
    !!s.http_token && s.status === 'connected'

  const availableServers: RemoteServerConfig[] = clonedServerIds?.length
    ? remoteServers.filter(s => isReady(s) && clonedServerIds.includes(s.id))
    : remoteServers.filter(isReady)

  const handleSelect = useCallback(
    (serverId: string | null) => {
      onOpenChange(false)
      onSelect(serverId)
    },
    [onOpenChange, onSelect]
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>Run where?</DialogTitle>
          {projectName && (
            <DialogDescription>
              Choose where to run <span className="font-medium">{projectName}</span>
            </DialogDescription>
          )}
        </DialogHeader>

        <div className="flex flex-col gap-2 py-2">
          <Button
            variant="outline"
            className="h-auto justify-start gap-3 px-4 py-3"
            onClick={() => handleSelect(null)}
          >
            <Laptop className="size-4 shrink-0 text-muted-foreground" />
            <div className="text-left">
              <div className="text-sm font-medium">Local</div>
              <div className="text-xs text-muted-foreground">Run on this machine</div>
            </div>
          </Button>

          {availableServers.map(server => (
            <Button
              key={server.id}
              variant="outline"
              className="h-auto justify-start gap-3 px-4 py-3"
              onClick={() => handleSelect(server.id)}
            >
              <Server className="size-4 shrink-0 text-muted-foreground" />
              <div className="text-left">
                <div className="text-sm font-medium">{server.name}</div>
                <div className="text-xs text-muted-foreground">
                  {server.host}
                </div>
              </div>
            </Button>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  )
}
