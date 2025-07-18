// Re-export all API schemas and types
export * from '../schemas/api.schemas.js'

// Re-export event-related schemas from event.schemas.js
export {
  EventListQuery,
  EventListResponse as EventListResponseFromSchema,
  EventStreamMessage,
  StreamStatusResponse,
} from '../schemas/event.schemas.js'

// Import specific types for the paginated responses that need Task and Event types
import type { Event } from './event.js'
import type { Task } from './task.js'

// Override the basic paginated responses with more specific ones that reference other types
export interface TaskListResponse {
  items: Task[]
  pagination: {
    limit: number
    offset: number
    total?: number
    hasMore?: boolean
  }
}

export interface EventListResponse {
  items: Event[]
  pagination?: {
    limit: number
    offset: number
    total?: number
    hasMore?: boolean
  }
}

// Re-export task schema types that are used in API
export type { GitStatus, MergeTaskResponse } from '../schemas/task.schemas.js'
