import { z } from 'zod'

export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  environmentId: z.string(),
  message: z.string(),
  branchName: z.string().optional(),
  start: z.boolean().default(true),
})

export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>

export const TaskStatusSchema = z.enum(['active', 'completed', 'error', 'closed'])

export const SimpleTaskResponseSchema = z.object({
  taskId: z.string(),
  status: TaskStatusSchema,
  message: z.string().optional(),
})

export type SimpleTaskResponse = z.infer<typeof SimpleTaskResponseSchema>

export const SendMessageRequestSchema = z.object({
  message: z.string().min(1),
})

export type SendMessageRequest = z.infer<typeof SendMessageRequestSchema>

export interface ApiError {
  code: string
  message: string
  details?: any
}

export interface PaginatedResponse<T> {
  items: T[]
  total: number
  page: number
  pageSize: number
  hasMore: boolean
}

export const PaginationQuerySchema = z.object({
  page: z.coerce.number().min(1).default(1),
  pageSize: z.coerce.number().min(1).max(100).default(20),
})

export type PaginationQuery = z.infer<typeof PaginationQuerySchema>

export const TaskListQuerySchema = PaginationQuerySchema.extend({
  status: z.enum(['active', 'completed', 'error', 'closed']).optional(),
  agentType: z.string().optional(),
  environmentId: z.string().optional(),
})

export type TaskListQuery = z.infer<typeof TaskListQuerySchema>

export const FileSearchQuerySchema = z.object({
  query: z.string(),
  filePattern: z.string().optional(),
  maxResults: z.number().min(1).max(1000).default(100),
})

export type FileSearchQuery = z.infer<typeof FileSearchQuerySchema>

export const WorkspaceActionRequestSchema = z.object({
  action: z.enum(['start', 'stop', 'restart', 'clean']),
})

export type WorkspaceActionRequest = z.infer<typeof WorkspaceActionRequestSchema>
