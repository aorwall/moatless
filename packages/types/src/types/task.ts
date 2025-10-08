// Re-export types from schemas
export type {
  GitDiffFile,
  GitDiffResponse,
  TaskIteration,
  TaskResponse as Task,
} from '../schemas/task.schemas.js'
// Re-export git types from workspace schemas
export type { GitChanges } from '../schemas/workspace.schemas.js'
// Re-export from API for convenience
export type { CreateTaskRequest, GitStatus } from './api.js'
