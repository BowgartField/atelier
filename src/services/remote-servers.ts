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
  RemoteServerStatus,
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

function setRemoteServerStatus(
  queryClient: ReturnType<typeof useQueryClient>,
  serverId: string,
  status: RemoteServerStatus
) {
  queryClient.setQueriesData<RemoteServerConfig[]>(
    { queryKey: remoteServersQueryKeys.all },
    servers =>
      servers?.map(server =>
        server.id === serverId ? { ...server, status } : server
      )
  )
}

async function connectAndRegister(
  serverId: string,
  queryClient: ReturnType<typeof useQueryClient>
): Promise<RemoteConnection> {
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
    // Tunnel opened but the webview transport failed to register. Tear the
    // tunnel down and mark the server errored so status polling doesn't keep
    // repainting the card as connected.
    await invoke('report_remote_connection_failure', {
      serverId,
      error: error instanceof Error ? error.message : String(error),
    }).catch(() => undefined)
    throw error
  }
  queryClient.setQueriesData<RemoteServerConfig[]>(
    { queryKey: remoteServersQueryKeys.all },
    servers =>
      servers?.map(server =>
        server.id === serverId
          ? {
              ...server,
              status: 'connected',
              http_token: connection.token,
            }
          : server
      )
  )
  await queryClient.invalidateQueries({ queryKey: ['projects'] })
  return connection
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
  const queryClient = useQueryClient()
  return useRemoteServerMutation<RemoteServerInput, RemoteServerConfig>(
    async config => {
      const server = await invoke<RemoteServerConfig>('add_remote_server', {
        config,
      })
      if (
        server.http_token &&
        (server.status === 'reachable' || server.status === 'connected')
      ) {
        try {
          await connectAndRegister(server.id, queryClient)
        } catch {
          server.status = 'error'
        }
      }
      return server
    },
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
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (serverId: string) => {
      const result = await invoke<RemoteConnectionTest>('test_remote_server', {
        serverId,
      })
      if (!result.success) throw new Error(result.message)
      return result
    },
    retry: false,
    onMutate: serverId => {
      setRemoteServerStatus(queryClient, serverId, 'connecting')
    },
    onError: (_error, serverId) => {
      setRemoteServerStatus(queryClient, serverId, 'error')
    },
    onSettled: (_data, error) => {
      if (!error) {
        queryClient.invalidateQueries({ queryKey: remoteServersQueryKeys.all })
      }
    },
  })
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
  return useMutation({
    mutationFn: (serverId: string) => connectAndRegister(serverId, queryClient),
    retry: false,
    onMutate: serverId => {
      setRemoteServerStatus(queryClient, serverId, 'connecting')
    },
    onError: (_error, serverId) => {
      setRemoteServerStatus(queryClient, serverId, 'error')
    },
    onSettled: (_data, error) => {
      if (!error) {
        queryClient.invalidateQueries({ queryKey: remoteServersQueryKeys.all })
      }
    },
  })
}

export function useDisconnectRemoteServer() {
  return useRemoteServerMutation<string, undefined>(async serverId => {
    await invoke('disconnect_remote_server', { serverId })
    unregisterRemoteTransport(serverId)
    return undefined
  })
}
