/** Command Code CLI management service. */

import { useQuery } from '@tanstack/react-query'
import { invoke } from '@/lib/transport'
import { logger } from '@/lib/logger'
import type {
  CommandCodeAuthStatus,
  CommandCodeCliStatus,
  CommandCodeInstallCommand,
} from '@/types/commandcode-cli'
import { hasBackend } from '@/lib/environment'

const isTauri = hasBackend

export const commandcodeCliQueryKeys = {
  all: ['commandcode-cli'] as const,
  status: () => [...commandcodeCliQueryKeys.all, 'status'] as const,
  auth: () => [...commandcodeCliQueryKeys.all, 'auth'] as const,
  installCommand: () => [...commandcodeCliQueryKeys.all, 'install-command'] as const,
}

export function useCommandCodePathDetection(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: [...commandcodeCliQueryKeys.all, 'path-detection'],
    queryFn: async (): Promise<{
      found: boolean
      path: string | null
      version: string | null
      package_manager: string | null
    }> => {
      if (!isTauri()) {
        return { found: false, path: null, version: null, package_manager: null }
      }
      try {
        return await invoke('detect_commandcode_in_path')
      } catch (error) {
        logger.debug('Command Code path detection failed', { error })
        return { found: false, path: null, version: null, package_manager: null }
      }
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 30,
    gcTime: 1000 * 60 * 60,
  })
}

export function useCommandCodeCliStatus(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: commandcodeCliQueryKeys.status(),
    queryFn: async (): Promise<CommandCodeCliStatus> => {
      if (!isTauri()) return { installed: false, version: null, path: null }
      try {
        return await invoke<CommandCodeCliStatus>('check_commandcode_cli_installed')
      } catch (error) {
        logger.error('Failed to check Command Code CLI status', { error })
        return { installed: false, version: null, path: null }
      }
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 5,
    gcTime: 1000 * 60 * 10,
    refetchInterval: 1000 * 60 * 60,
  })
}

export function useCommandCodeCliAuth(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: commandcodeCliQueryKeys.auth(),
    queryFn: async (): Promise<CommandCodeAuthStatus> => {
      if (!isTauri()) {
        return { authenticated: false, error: 'Not in Tauri context', timed_out: false }
      }
      try {
        return await invoke<CommandCodeAuthStatus>('check_commandcode_cli_auth')
      } catch (error) {
        logger.error('Failed to check Command Code CLI auth', { error })
        return {
          authenticated: false,
          error: error instanceof Error ? error.message : String(error),
          timed_out: false,
        }
      }
    },
    enabled: options?.enabled ?? true,
    staleTime: 1000 * 60 * 5,
    gcTime: 1000 * 60 * 10,
  })
}

export async function getCommandCodeInstallCommand(): Promise<CommandCodeInstallCommand> {
  return invoke<CommandCodeInstallCommand>('get_commandcode_install_command')
}
