// Re-export types from schemas
export type {
  GitDiffFile,
  GitDiffResponse,
  TaskIteration,
  TaskResponse as Task,
} from '../schemas/task.schemas.js'

// Re-export from API for convenience
export type { CreateTaskRequest, GitChanges, GitStatus } from './api.js'
