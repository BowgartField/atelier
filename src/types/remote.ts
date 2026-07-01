export type RemoteServerAuth =
  | { type: 'ssh_key_path'; path: string; passphrase?: string | null }
  | { type: 'password'; password: string }

export type RemoteServerStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'provisioning'
  | 'error'

export interface RemoteServerConfig {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth: RemoteServerAuth
  default: boolean
  remote_port: number
  status: RemoteServerStatus
  http_token?: string | null
  installed_version?: string | null
}

export type RemoteServerInput = Pick<
  RemoteServerConfig,
  'name' | 'host' | 'port' | 'username' | 'auth' | 'default' | 'remote_port'
>

export interface RemoteConnectionTest {
  success: boolean
  message: string
  hostname?: string | null
  os?: string | null
  architecture?: string | null
}

export interface ProvisionResult {
  success: boolean
  version: string
  remote_port: number
  service_name: string
}

export interface RemoteJeanVersionInfo {
  version: string
  published_at: string
  prerelease: boolean
}

export interface RemoteProvisionProgress {
  server_id: string
  stage: string
  message: string
  percent: number
}

export interface RemoteProvisionLogLine {
  server_id: string
  stream: 'stdout' | 'stderr' | 'system'
  line: string
}

export interface RemoteConnection {
  server_id: string
  local_port: number
  remote_port: number
  token: string
  url: string
}

export interface RemoteServerStatusInfo {
  server_id: string
  status: RemoteServerStatus
  local_port?: number | null
  remote_port: number
  last_error?: string | null
}

export interface LocalToolStatus {
  claude_cli: boolean
  gh_cli: boolean
}

export interface ToolsToInstall {
  claudeCli: boolean
  ghCli: boolean
}
