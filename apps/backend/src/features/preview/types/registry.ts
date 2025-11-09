import type { ServerStatusType } from './status.js'

export interface PreviewServerRecord {
  serverId: string
  status: ServerStatusType
  port?: number
  url?: string
  proxyUrl?: string
  hostname?: string
  pid?: number
  inspectorEnabled?: boolean
  error?: string
  dirtyContext?: boolean
  metadata?: Record<string, unknown>
  createdAt: Date
  updatedAt: Date
}

export type PreviewServerSnapshot = Omit<
  PreviewServerRecord,
  'serverId' | 'createdAt' | 'updatedAt'
>

export type PreviewServerUpdate = {
  [K in keyof PreviewServerSnapshot]?: PreviewServerSnapshot[K] | undefined
}

export interface ServerStatusChangeInfo {
  status: ServerStatusType
  port?: number
  pid?: number
  url?: string
  proxyUrl?: string
  error?: string
  uptimeSeconds?: number
}

export interface PlaywrightActionInfo {
  type: string
  url: string
  mode: 'idle' | 'user' | 'agent'
  timestamp: Date
}

export interface CodeChangeInfo {
  toolName: string
  timestamp: Date
  filePaths?: string[]
}

export interface PreviewServerLifecycleSink {
  register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void>
  update(serverId: string, changes: PreviewServerUpdate): Promise<void>
  remove(serverId: string): Promise<void>
  onStatusChange?(serverId: string, statusInfo: ServerStatusChangeInfo): Promise<void>
  onPlaywrightAction?(serverId: string, actionInfo: PlaywrightActionInfo): Promise<void>
  onPlaywrightModeChange?(serverId: string, mode: 'idle' | 'user' | 'agent'): Promise<void>
  onCodeChange?(serverId: string, changeInfo: CodeChangeInfo): Promise<void>
}
