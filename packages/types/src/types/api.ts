// Re-export all API schemas and types
export * from '../schemas/api.schemas.js'

// Import Task type for the custom interface
import type { Task } from './task.js'

// Custom interface that uses Task instead of TaskResponse
export interface TaskListResponse {
  items: Task[]
  pagination: {
    limit: number
    offset: number
    total: number | undefined
    hasMore: boolean | undefined
  }
}
