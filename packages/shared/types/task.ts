export interface TaskIteration {
  iterationNumber: number
  startTime: Date
  endTime?: Date
  status: 'active' | 'completed' | 'error' | 'failed'
  messageCount: number
  costUsd: number
  initialMessage?: string
  commandName?: string
  result?: string
  durationMs?: number
  gitDiff?: string
  merged?: boolean
  agentSessionId?: string
  model?: string
  gitSyncStatus?: {
    status: 'synced' | 'ahead' | 'behind' | 'diverged' | 'uncommitted_changes' | 'error'
    commitsAhead: number
    commitsBehind: number
    hasUncommittedChanges: boolean
    currentBranch: string
    baseBranch: string
    originExists: boolean
  }
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
