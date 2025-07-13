import type { GitStatus } from './api.js'

export interface TaskIteration {
  iterationNumber: number
  startTime: Date
  endTime?: Date
  status: 'created' | 'active' | 'completed' | 'error' | 'failed'
  messageCount: number
  costUsd: number
  contextTokens?: number
  initialMessage?: string
  commandName?: string
  result?: string
  durationMs?: number
  gitDiff?: string
  merged?: boolean
  agentSessionId?: string
  model?: string
  gitStatus?: GitStatus
  attachments?: Array<{
    fileId?: string // Optional for backward compatibility
    filename: string
    path: string // Absolute path to file
    mimeType: string
  }>
  error?: {
    message: string
    stack?: string
    timestamp: Date
  }
  willNotResume?: boolean // Indicates this iteration won't be resumed on next message
  // Runtime properties (not persisted)
  task?: Task
  taskId?: string
}

export interface Task {
  taskId: string
  createdAt: Date
  lastMessageAt: Date
  agentType: string
  repositoryId: string
  status: 'open' | 'running' | 'error' | 'merged' | 'closed'
  numTurns: number
  totalCostUsd: number
  iterations: TaskIteration[]
  workspacePath?: string
}

export interface GitDiffFile {
  oldFile?: string
  newFile?: string
  hunks: string[]
}

export interface GitDiffResponse {
  diff?: string
  files: GitDiffFile[]
}

export type { CreateTaskRequest } from './api.js'
