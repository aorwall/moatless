export interface TaskIteration {
  iterationNumber: number
  startTime: Date
  endTime?: Date
  status: 'active' | 'completed' | 'error'
  messageCount: number
  costUsd: number
  initialMessage?: string
  errorMessage?: string
  result?: string
  durationMs?: number
  gitDiff?: string
  merged?: boolean
}

export interface TaskMetadata {
  taskId: string
  createdAt: Date
  lastMessageAt: Date
  agentType: string
  environmentId: string
  status: 'active' | 'completed' | 'error' | 'closed'
  numTurns: number
  totalCostUsd: number
  iterations: TaskIteration[]
}

export interface TaskIterationResponse {
  iterationNumber: number
  startTime: Date
  endTime?: Date
  status: 'active' | 'completed' | 'error'
  messageCount: number
  costUsd: number
  initialMessage?: string
  errorMessage?: string
  result?: string
  durationMs?: number
  gitDiff?: GitDiffResponse
  merged?: boolean
}

export interface TaskResponse {
  taskId: string
  title?: string
  createdAt: Date
  lastMessageAt: Date
  agentType: string
  environmentId: string
  environmentName?: string
  status: 'active' | 'completed' | 'error' | 'closed'
  numTurns: number
  totalCostUsd: number
  isRunning: boolean
  workspaceStatus?: string
  iterations?: TaskIterationResponse[]
}

export interface TaskMessage {
  taskId: string
  message: string
  timestamp: Date
}

export interface TaskSummary {
  taskId: string
  title: string
  status: TaskMetadata['status']
  createdAt: Date
  agentType: string
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

export interface CreateTaskRequest {
  agentType: string
  environmentId: string
  message: string
  branchName?: string
  start?: boolean
}

// Zod schema for validation (can be imported from api.ts if needed)
import { z } from 'zod'

export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  environmentId: z.string(),
  message: z.string(),
  branchName: z.string().optional(),
  start: z.boolean().default(true),
})
