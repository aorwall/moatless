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

export interface PreviewServerLifecycleSink {
  register(serverId: string, snapshot: PreviewServerSnapshot): Promise<void>
  update(serverId: string, changes: PreviewServerUpdate): Promise<void>
  remove(serverId: string): Promise<void>
}
