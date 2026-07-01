import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { hasBackend } from '@/lib/environment'
import {
  invoke,
  registerRemoteTransport,
  unregisterRemoteTransport,
} from '@/lib/transport'
import type {
  ProvisionResult,
  RemoteConnection,
  RemoteConnectionTest,
  RemoteJeanVersionInfo,
  RemoteServerConfig,
  RemoteServerInput,
} from '@/types/remote'
import { preferencesQueryKeys } from './preferences'

export const remoteServersQueryKeys = {
  all: ['remote-servers'] as const,
  list: () => [...remoteServersQueryKeys.all, 'list'] as const,
  versions: () => [...remoteServersQueryKeys.all, 'versions'] as const,
}

export function useRemoteServers() {
  return useQuery({
    queryKey: remoteServersQueryKeys.list(),
    queryFn: () =>
      invoke<RemoteServerConfig[]>('list_remote_servers').then(
        servers => servers ?? []
      ),
    enabled: hasBackend(),
    refetchInterval: 3000,
    refetchIntervalInBackground: false,
  })
}

export function useRemoteJeanVersions(enabled: boolean) {
  return useQuery({
    queryKey: remoteServersQueryKeys.versions(),
    queryFn: () =>
      invoke<RemoteJeanVersionInfo[]>('list_remote_jean_versions').then(
        versions => versions ?? []
      ),
    enabled: enabled && hasBackend(),
    staleTime: 5 * 60_000,
  })
}

function useRemoteServerMutation<TVariables, TResult>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
  invalidatePreferences = false
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn,
    retry: false,
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: remoteServersQueryKeys.all,
      })
      if (invalidatePreferences) {
        queryClient.invalidateQueries({
          queryKey: preferencesQueryKeys.all,
        })
      }
    },
  })
}

export function useAddRemoteServer() {
  return useRemoteServerMutation<RemoteServerInput, RemoteServerConfig>(
    config => invoke('add_remote_server', { config }),
    true
  )
}

export function useUpdateRemoteServer() {
  return useRemoteServerMutation<
    { serverId: string; config: RemoteServerInput },
    RemoteServerConfig
  >(
    ({ serverId, config }) =>
      invoke('update_remote_server', { serverId, config }),
    true
  )
}

export function useRemoveRemoteServer() {
  return useRemoteServerMutation<string, undefined>(async serverId => {
    await invoke('remove_remote_server', { serverId })
    return undefined
  }, true)
}

export function useTestRemoteServer() {
  return useRemoteServerMutation<string, RemoteConnectionTest>(serverId =>
    invoke('test_remote_server', { serverId })
  )
}

export function useProvisionRemoteServer() {
  return useRemoteServerMutation<
    { serverId: string; version?: string },
    ProvisionResult
  >(({ serverId, version }) =>
    invoke('provision_remote_server', { serverId, version })
  )
}

export function useConnectRemoteServer() {
  const queryClient = useQueryClient()
  return useRemoteServerMutation<string, RemoteConnection>(async serverId => {
    const connection = await invoke<RemoteConnection>('connect_remote_server', {
      serverId,
    })
    try {
      await registerRemoteTransport(
        connection.server_id,
        connection.local_port,
        connection.token
      )
    } catch (error) {
      await invoke('disconnect_remote_server', { serverId }).catch(
        () => undefined
      )
      throw error
    }
    await queryClient.invalidateQueries({
      queryKey: ['projects', 'worktrees'],
    })
    return connection
  })
}

export function useDisconnectRemoteServer() {
  return useRemoteServerMutation<string, undefined>(async serverId => {
    await invoke('disconnect_remote_server', { serverId })
    unregisterRemoteTransport(serverId)
    return undefined
  })
}
