// Re-export all API schemas and types
export * from '../schemas/api.schemas.js'

// Import specific types for the paginated responses that need Task and Event types
import type { Event } from './event.js'
import type { Task } from './task.js'

// Override the basic paginated responses with more specific ones that reference other types
export interface TaskListResponse {
  tasks: Task[]
  items: Task[]
  pagination: {
    limit: number
    offset: number
    total?: number
  }
}

export interface EventListResponse {
  events: Event[]
  items: Event[]
}

// Re-export task schema types that are used in API
export type { GitStatus, MergeTaskResponse } from '../schemas/task.schemas.js'

// ============================================
// Legacy Support / Deprecated Types
// ============================================

// @deprecated Use CreateTaskRequest instead
export interface AgentTriggerRequest {
  agentType: string
  repositoryId: string
  message?: string
  branch?: string // Maps to branchName in CreateTaskRequest
  branchName?: string
  originalBranch?: string
  start?: boolean
  model?: string
  filePaths?: string[]
}

// @deprecated Use SimpleTaskResponse instead
export interface AgentTriggerResponse {
  taskId: string
  status: string
  streamUrl?: string
}
