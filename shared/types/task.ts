export interface TaskIteration {
  iterationNumber: number
  startTime: Date
  endTime?: Date
  status: 'active' | 'completed' | 'error' | 'failed'
  messageCount: number
  costUsd: number
  initialMessage?: string
  result?: string
  durationMs?: number
  gitDiff?: string
  merged?: boolean
  agentSessionId?: string
  error?: {
    message: string
    stack?: string
    timestamp: Date
  }
  // Runtime properties (not persisted)
  task?: Task
  taskId?: string
}

export interface Task {
  taskId: string
  createdAt: Date
  lastMessageAt: Date
  agentType: string
  environmentId: string
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
