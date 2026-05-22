/** Types for Command Code CLI management. */

export interface CommandCodeCliStatus {
  installed: boolean
  version: string | null
  path: string | null
}

export interface CommandCodeAuthStatus {
  authenticated: boolean
  error: string | null
  timed_out?: boolean
}

export interface CommandCodeInstallCommand {
  command: string
  args: string[]
  description: string
}
