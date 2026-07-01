import type { RemoteJeanVersionInfo } from '@/types/remote'
import { FALLBACK_APP_VERSION } from './app-version'

/**
 * Picks the version to provision by default: the desktop's own version if
 * it has a published release, otherwise the latest published release (dev
 * builds run ahead of the next tag, which has no release artifact yet).
 */
export function resolveDefaultProvisionVersion(
  versions: RemoteJeanVersionInfo[] | undefined
): string {
  const latest = versions?.[0]
  if (!latest) return FALLBACK_APP_VERSION
  if (versions.some(v => v.version === FALLBACK_APP_VERSION)) {
    return FALLBACK_APP_VERSION
  }
  return latest.version
}
