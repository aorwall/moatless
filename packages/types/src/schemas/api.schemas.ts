import { z } from 'zod'
import { TaskStatusSchema } from './base.schemas.js'
import { TaskResponseSchema } from './task.schemas.js'

// ============================================
// Base Types and Enums
// ============================================

// ============================================
// Request Schemas
// ============================================

// Note: CreateTaskRequestSchema and CreateVibeTaskRequestSchema moved to task.schemas.ts

// Send Message
export const SendMessageRequestSchema = z.object({
  message: z.string().min(1),
  resume: z.boolean().optional().default(true),
  model: z.string().optional(),
  fileIds: z.array(z.string()).optional(), // Deprecated, use filePaths
  filePaths: z.array(z.string()).optional(), // Absolute file paths
  commandName: z.string().optional(),
  agentMode: z.enum(['edit', 'plan']).optional().default('edit'),
})

// General Upload (not tied to task)
export const UploadFileResponseSchema = z.object({
  id: z.string(),
  filename: z.string(),
  path: z.string(), // Relative path like /uploads/{randomId}/filename.ext
  absolutePath: z.string(), // Absolute path in workspace
  size: z.number(),
  mimeType: z.string(),
  uploadedAt: z.string(),
})

// Update Task
export const UpdateTaskRequestSchema = z.object({
  status: TaskStatusSchema.optional(),
})

// ============================================
// Response Schemas
// ============================================

// Simple Task Response (for create/update operations)
export const SimpleTaskResponseSchema = z.object({
  taskId: z.string(),
  status: z.string(),
  task: TaskResponseSchema.optional(),
})

// Send Message Response
export const SendMessageResponseSchema = z.object({
  status: z.string(),
  timestamp: z.string(),
})

// Delete Task Response
export const DeleteTaskResponseSchema = z.object({
  status: z.string(),
})

// Update Task Response
export const UpdateTaskResponseSchema = z.object({
  status: z.string(),
})

// Stop Agent Response
export const StopAgentResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
})

// Server Logs Response
export const ServerLogsResponseSchema = z.object({
  logs: z.string(),
  lines: z.number(),
})

// Task Commands Response
export const TaskCommandsResponseSchema = z.object({
  commands: z.array(
    z.object({
      name: z.string(),
      description: z.string(),
      prompt: z.string(),
      conditions: z.any().optional(),
      icon: z.string().optional(),
      variant: z.string().optional(),
      active: z.boolean().optional(),
    }),
  ),
})

// Start Task Response
export const StartTaskResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
})

// Close Task Response
export const CloseTaskResponseSchemaAPI = z.object({
  status: z.string(),
  taskId: z.string(),
})

// MCP JSON-RPC Response (generic)
export const McpResponseSchema = z.record(z.string(), z.unknown())

// Git Sync Status with Origin
export const GitSyncStatusSchema = z.enum([
  'synced',
  'ahead',
  'behind',
  'diverged',
  'uncommitted_changes',
  'error',
])

// Git Changes Schema - Working directory changes (not diff to base branch)
export const GitChangesSchema = z.object({
  currentBranch: z.string(),
  status: z.string(), // Simple status like 'clean' or 'modified'
  additions: z.number(), // Lines added in working directory
  deletions: z.number(), // Lines deleted in working directory
  modifications: z.number(), // Files modified in working directory
})

// ============================================
// Query Schemas
// ============================================

// Pagination Query
export const PaginationQuerySchema = z.object({
  limit: z.coerce.number().min(1).max(100).default(20),
  offset: z.coerce.number().min(0).default(0),
})

// Task List Query
export const TaskListQuerySchema = PaginationQuerySchema.extend({
  status: TaskStatusSchema.optional(),
})

// File Search Query
export const FileSearchQuerySchema = z.object({
  query: z.string(),
  filePattern: z.string().optional(),
  maxResults: z.number().min(1).max(1000).default(100),
})

// ============================================
// Derived Types
// ============================================
// Re-export task request types from task.schemas.ts for backward compatibility
export type {
  CreateTaskRequest,
  CreateVibeTaskRequest,
} from './task.schemas.js'
export type SendMessageRequest = z.infer<typeof SendMessageRequestSchema>
export type UploadFileResponse = z.infer<typeof UploadFileResponseSchema>
export type UpdateTaskRequest = z.infer<typeof UpdateTaskRequestSchema>
export type SimpleTaskResponse = z.infer<typeof SimpleTaskResponseSchema>
export type SendMessageResponse = z.infer<typeof SendMessageResponseSchema>
export type DeleteTaskResponse = z.infer<typeof DeleteTaskResponseSchema>
export type UpdateTaskResponse = z.infer<typeof UpdateTaskResponseSchema>
export type StopAgentResponse = z.infer<typeof StopAgentResponseSchema>
export type ServerLogsResponse = z.infer<typeof ServerLogsResponseSchema>
export type TaskCommandsResponse = z.infer<typeof TaskCommandsResponseSchema>
export type GitSyncStatus = z.infer<typeof GitSyncStatusSchema>
export type GitChanges = z.infer<typeof GitChangesSchema>
export type PaginationQuery = z.infer<typeof PaginationQuerySchema>
export type TaskListQuery = z.infer<typeof TaskListQuerySchema>
export type FileSearchQuery = z.infer<typeof FileSearchQuerySchema>
export type PaginationInfo = z.infer<typeof PaginationInfoSchema>

// ============================================
// Pagination Response Types
// ============================================

export const PaginationInfoSchema = z.object({
  limit: z.number(),
  offset: z.number(),
  total: z.number().or(z.undefined()),
  hasMore: z.boolean().or(z.undefined()),
})

export function createPaginatedResponseSchema<T extends z.ZodType>(itemSchema: T) {
  return z.object({
    items: z.array(itemSchema),
    pagination: PaginationInfoSchema,
  })
}

export interface PaginatedResponse<T> {
  items: T[]
  pagination: PaginationInfo
}

// ============================================
// Error Schemas
// ============================================

// API Error Response Schema (matching the TypeScript interfaces)
export const ApiResponseErrorSchema = z.object({
  code: z.string(),
  message: z.string(),
  details: z.record(z.string(), z.unknown()).optional(),
})

export const ApiErrorResponseSchema = z.object({
  error: ApiResponseErrorSchema,
  requestId: z.string().optional(),
})

// Validation Error Schema
export const ApiValidationErrorSchema = ApiResponseErrorSchema.extend({
  code: z.literal('VALIDATION_ERROR'),
  details: z.object({
    errors: z.union([
      z.record(z.string(), z.string()),
      z.array(
        z.object({
          path: z.union([z.string(), z.array(z.string())]),
          message: z.string(),
        }),
      ),
    ]),
  }),
})

// Not Found Error Schema
export const NotFoundErrorSchema = ApiResponseErrorSchema.extend({
  code: z.enum(['NOT_FOUND', 'ENVIRONMENT_NOT_FOUND', 'REPOSITORY_NOT_FOUND', 'TASK_NOT_FOUND']),
  details: z
    .object({
      id: z.string().optional(),
      resource: z.string().optional(),
    })
    .optional(),
})

// Internal Server Error Schema
export const InternalServerErrorSchema = ApiResponseErrorSchema.extend({
  code: z.literal('INTERNAL_SERVER_ERROR'),
  details: z
    .object({
      error: z.string().optional(),
    })
    .optional(),
})

// ============================================
// Error Types
// ============================================

export interface BaseApiError {
  code: string
  message: string
  details?: unknown
}
