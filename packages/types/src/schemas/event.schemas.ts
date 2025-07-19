import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

// ============================================
// Event Types and Enums
// ============================================

// Event type enum using string literals (following schema design patterns)
export const EventTypeSchema = z.enum([
  'message',
  'start',
  'stop',
  'result',
  'error',
  'workspace',
  'environment',
  'server',
  'tool',
  'completion',
  'task_status',
])

// ============================================
// Base Event Schema
// ============================================

export const BaseEventSchema = z.object({
  eventId: z.string(),
  agentType: z.string(),
  type: EventTypeSchema,
  subType: z.string().optional(),
  taskId: z.string(),
  iterationId: z.number().optional(),
  timestamp: dateSchema.optional(),
  isError: z.boolean(),
  contextTokens: z.number().optional(),
  numTurns: z.number().optional(),
  totalCost: z.number().optional(),
  agentSessionId: z.string().optional(),
})

// ============================================
// Event Payload Schemas
// ============================================

// Generic event payload - using record for flexibility
export const EventPayloadSchema = z.record(z.string(), z.unknown())

// Claude Code SDK Message Schema (referenced in MessageEvent)
// Using z.any() since ClaudeCodeSDKMessage is imported from external package
export const ClaudeCodeSDKMessageSchema = z.any() // Actual type comes from @anthropic-ai/claude-code

// SDK Result Message Schema (referenced in ResultEvent)
// Using z.any() since SDKResultMessage is imported from external package
export const SDKResultMessageSchema = z.any() // Actual type comes from @anthropic-ai/claude-code

// Error payload schema
export const ErrorPayloadSchema = z.object({
  error: z.string(),
  stackTrace: z.string().optional(),
  code: z.string().optional(),
})

// Task status payload schema
export const TaskStatusPayloadSchema = z.object({
  status: z.string(),
  previousStatus: z.string(),
  iterationNumber: z.number().optional(),
})

// ============================================
// Specific Event Schemas
// ============================================

// Generic event with payload
export const EventSchema = BaseEventSchema.extend({
  payload: EventPayloadSchema,
})

// Message event
export const MessageEventSchema = BaseEventSchema.extend({
  type: z.literal('message'),
  payload: ClaudeCodeSDKMessageSchema,
})

// Error event
export const ErrorEventSchema = BaseEventSchema.extend({
  type: z.literal('error'),
  isError: z.literal(true),
  payload: ErrorPayloadSchema,
})

// Workspace event
export const WorkspaceEventSchema = BaseEventSchema.extend({
  type: z.literal('workspace'),
  payload: EventPayloadSchema,
})

// Environment event
export const EnvironmentEventSchema = BaseEventSchema.extend({
  type: z.literal('environment'),
  payload: EventPayloadSchema,
})

// Server event
export const ServerEventSchema = BaseEventSchema.extend({
  type: z.literal('server'),
  payload: EventPayloadSchema,
})

// Result event
export const ResultEventSchema = BaseEventSchema.extend({
  type: z.literal('result'),
  payload: SDKResultMessageSchema,
})

// Task status event
export const TaskStatusEventSchema = BaseEventSchema.extend({
  type: z.literal('task_status'),
  subType: z.string(), // The status that was changed to
  payload: TaskStatusPayloadSchema,
})

// Union of all specific event types
export const SpecificEventSchema = z.discriminatedUnion('type', [
  MessageEventSchema,
  ErrorEventSchema,
  WorkspaceEventSchema,
  EnvironmentEventSchema,
  ServerEventSchema,
  ResultEventSchema,
  TaskStatusEventSchema,
])

// ============================================
// Event Stream Schemas (moved from api.schemas.ts)
// ============================================

// Event Stream Message Schema (for NDJSON format)
export const EventStreamMessageSchema = z.object({
  type: z.string(),
  timestamp: z.string(),
  data: z.any().optional(), // Event-specific data
})

// Stream Status Response Schema (moved from api.schemas.ts)
export const StreamStatusResponseSchema = z.object({
  globalConnections: z.number(),
  globalEventCount: z.number(),
  taskConnections: z.record(z.string(), z.number()),
  taskEventCounts: z.record(z.string(), z.number()),
  totalConnections: z.number(),
  totalEventCount: z.number(),
})

// ============================================
// Worker Event Schemas (for worker.ts)
// ============================================

// Worker emits events directly - no wrapper needed since Event already has taskId
// Using EventSchema directly for the request body

export const WorkerEventResponseSchema = z.object({
  success: z.boolean(),
  message: z.string().optional(),
})

// ============================================
// Query Schemas (moved from task.schemas.ts)
// ============================================

export const EventTypeQuerySchema = z.object({
  type: z.string().optional(),
})

export const EventListQuerySchema = z.object({
  limit: z.coerce.number().min(1).max(100).default(20),
  offset: z.coerce.number().min(0).default(0),
  type: z.string().optional(),
})

// ============================================
// Response Schemas
// ============================================

export const EventListResponseSchema = z.object({
  items: z.array(EventSchema), // Using proper Event schema instead of z.any()
  pagination: z
    .object({
      limit: z.number(),
      offset: z.number(),
      total: z.number().optional(),
      hasMore: z.boolean().optional(),
    })
    .optional(),
})

// ============================================
// Derived Types
// ============================================

export type EventType = z.infer<typeof EventTypeSchema>
export type BaseEvent = z.infer<typeof BaseEventSchema>
export type EventPayload = z.infer<typeof EventPayloadSchema>
export type Event = z.infer<typeof EventSchema>
// Re-export types from claude.js
export type { ClaudeCodeSDKMessage, SDKResultMessage } from '../types/claude.js'
export type MessageEvent = z.infer<typeof MessageEventSchema>
export type ErrorEvent = z.infer<typeof ErrorEventSchema>
export type WorkspaceEvent = z.infer<typeof WorkspaceEventSchema>
export type EnvironmentEvent = z.infer<typeof EnvironmentEventSchema>
export type ServerEvent = z.infer<typeof ServerEventSchema>
export type ResultEvent = z.infer<typeof ResultEventSchema>
export type TaskStatusEvent = z.infer<typeof TaskStatusEventSchema>
export type SpecificEvent = z.infer<typeof SpecificEventSchema>
export type EventStreamMessage = z.infer<typeof EventStreamMessageSchema>
export type StreamStatusResponse = z.infer<typeof StreamStatusResponseSchema>
export type WorkerEventResponse = z.infer<typeof WorkerEventResponseSchema>
export type EventTypeQuery = z.infer<typeof EventTypeQuerySchema>
export type EventListQuery = z.infer<typeof EventListQuerySchema>
export type EventListResponse = z.infer<typeof EventListResponseSchema>
