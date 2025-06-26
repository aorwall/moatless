import { z } from 'zod'
import type { Event } from './event.js'
import type { Task } from './task.js'
import { WorkspaceStatus } from './workspace.js'

// ============================================
// Base Types and Enums
// ============================================

export const TaskStatusSchema = z.enum(['active', 'completed', 'error', 'closed'])
export type TaskStatus = z.infer<typeof TaskStatusSchema>

// ============================================
// Request Schemas and Types
// ============================================

// Create Task
export const CreateTaskRequestSchema = z.object({
  agentType: z.string(),
  environmentId: z.string(),
  message: z.string().optional(),
  branchName: z.string().optional(),
  start: z.boolean().optional(),
})
export type CreateTaskRequest = z.infer<typeof CreateTaskRequestSchema>

// Send Message
export const SendMessageRequestSchema = z.object({
  message: z.string().min(1),
})
export type SendMessageRequest = z.infer<typeof SendMessageRequestSchema>

// Update Task
export const UpdateTaskRequestSchema = z.object({
  status: TaskStatusSchema.optional(),
})
export type UpdateTaskRequest = z.infer<typeof UpdateTaskRequestSchema>

// ============================================
// Response Schemas and Types
// ============================================

// Simple Task Response (for create/update operations)
export const SimpleTaskResponseSchema = z.object({
  taskId: z.string(),
  status: z.string(),
})
export type SimpleTaskResponse = z.infer<typeof SimpleTaskResponseSchema>

// Send Message Response
export const SendMessageResponseSchema = z.object({
  status: z.string(),
  timestamp: z.string(),
})
export type SendMessageResponse = z.infer<typeof SendMessageResponseSchema>

// Delete Task Response
export const DeleteTaskResponseSchema = z.object({
  status: z.string(),
})
export type DeleteTaskResponse = z.infer<typeof DeleteTaskResponseSchema>

// Update Task Response
export const UpdateTaskResponseSchema = z.object({
  status: z.string(),
})
export type UpdateTaskResponse = z.infer<typeof UpdateTaskResponseSchema>

// Stop Agent Response
export const StopAgentResponseSchema = z.object({
  status: z.string(),
  taskId: z.string(),
})
export type StopAgentResponse = z.infer<typeof StopAgentResponseSchema>

// Merge Task Response
export const MergeTaskResponseSchema = z.object({
  status: z.string(),
  message: z.string(),
})
export type MergeTaskResponse = z.infer<typeof MergeTaskResponseSchema>

// Server Logs Response
export const ServerLogsResponseSchema = z.object({
  logs: z.string(),
  lines: z.number(),
})
export type ServerLogsResponse = z.infer<typeof ServerLogsResponseSchema>

// Workspace Status Response
export const WorkspaceStatusResponseSchema = z.object({
  status: z.nativeEnum(WorkspaceStatus),
  hasUncommittedChanges: z.boolean(),
})
export type WorkspaceStatusResponse = z.infer<typeof WorkspaceStatusResponseSchema>

// Agent Status Response
export const AgentStatusResponseSchema = z.object({
  status: z.enum(['idle', 'running', 'preparing', 'error']),
  sessionId: z.string().nullable(),
  startedAt: z.string().nullable(),
  currentIteration: z.number().nullable(),
  workspaceStatus: z.nativeEnum(WorkspaceStatus),
  canExecute: z.boolean(),
})
export type AgentStatusResponse = z.infer<typeof AgentStatusResponseSchema>

// ============================================
// Query Schemas and Types
// ============================================

// Pagination Query
export const PaginationQuerySchema = z.object({
  limit: z.coerce.number().min(1).max(100).default(20),
  offset: z.coerce.number().min(0).default(0),
})
export type PaginationQuery = z.infer<typeof PaginationQuerySchema>

// Task List Query
export const TaskListQuerySchema = PaginationQuerySchema.extend({
  status: TaskStatusSchema.optional(),
})
export type TaskListQuery = z.infer<typeof TaskListQuerySchema>

// Event List Query
export const EventListQuerySchema = PaginationQuerySchema.extend({
  type: z.string().optional(),
})
export type EventListQuery = z.infer<typeof EventListQuerySchema>

// File Search Query
export const FileSearchQuerySchema = z.object({
  query: z.string(),
  filePattern: z.string().optional(),
  maxResults: z.number().min(1).max(1000).default(100),
})
export type FileSearchQuery = z.infer<typeof FileSearchQuerySchema>

// ============================================
// Paginated Response Types
// ============================================

export interface PaginationInfo {
  limit: number
  offset: number
  total?: number
  hasMore?: boolean
}

export interface PaginatedResponse<T> {
  items: T[]
  pagination: PaginationInfo
}

// Specific paginated responses
export interface TaskListResponse extends PaginatedResponse<Task> {
  tasks: Task[]
}

export interface EventListResponse {
  events: Event[]
  items: Event[]
}

// ============================================
// Error Types
// ============================================

export interface ApiError {
  code: string
  message: string
  details?: any
}

// ============================================
// Legacy Support / Deprecated Types
// ============================================

// @deprecated Use CreateTaskRequest instead
export interface AgentTriggerRequest extends CreateTaskRequest {
  branch?: string // Maps to branchName in CreateTaskRequest
}

// @deprecated Use SimpleTaskResponse instead
export interface AgentTriggerResponse extends SimpleTaskResponse {
  streamUrl?: string
}
