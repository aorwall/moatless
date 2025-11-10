import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { EnvironmentStatusSchema } from './workspace.schemas.js'

// ============================================
// Event Types and Enums
// ============================================

// Event type enum using string literals (following schema design patterns)
// Consolidated from 12 types to 6 logical types
export const EventTypeSchema = z.enum([
  'task', // Task lifecycle (replaces task_status)
  'taskIteration', // Task iteration execution (new)
  'workspace', // Workspace operations (unchanged)
  'environment', // Environment setup/status (unchanged)
  'agent', // Agent lifecycle + messages + results (consolidated)
  'server', // Server management (unchanged)
  'navigation', // Preview navigation events (new)
  'preview', // Playwright preview interaction events (new)
])

// ============================================
// Base Event Schema
// ============================================

export const BaseEventSchema = z.object({
  eventId: z.string(),
  type: EventTypeSchema,
  subType: z.string().optional(),
  taskId: z.string(),
  timestamp: dateSchema.optional(),
  isError: z.boolean(),
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

// Task payload schema
export const TaskPayloadSchema = z.object({
  status: z.string(),
  previousStatus: z.string(),
})

// Task iteration payload schema
export const TaskIterationPayloadSchema = z.object({
  status: z.string(),
  previousStatus: z.string(),
  iterationNumber: z.number(),
  screenshotFileId: z.string().optional(),
  contextTokens: z.number().optional(),
  agentSessionId: z.string().optional(),
})

// Server event payload schema
export const ServerEventPayloadSchema = z.object({
  status: z.enum(['stopped', 'installing', 'starting', 'started', 'failed']),
  previousStatus: z.enum(['stopped', 'installing', 'starting', 'started', 'failed']).optional(),
  port: z.number().optional(),
  pid: z.number().optional(),
  url: z.string().optional(),
  error: z.string().optional(),
  uptimeSeconds: z.number().optional(),
})

// Navigation event payload schema
export const NavigationEventPayloadSchema = z.object({
  path: z.string(),
  previousPath: z.string(),
  url: z.string().optional(),
  timestamp: z.date().optional(),
})

// Preview event payload schema
export const PreviewEventPayloadSchema = z.object({
  actionType: z.enum(['navigate', 'click', 'evaluate', 'fillForm', 'snapshot']).optional(),
  url: z.string().optional(),
  mode: z.enum(['idle', 'user', 'agent']).optional(),
  timestamp: z.date().optional(),
  toolName: z.string().optional(),
  filePaths: z.array(z.string()).optional(),
})

// Agent event payload schema for agent-specific fields
export const AgentEventPayloadSchema = z.object({
  contextTokens: z.number().optional(),
  numTurns: z.number().optional(),
  totalCost: z.number().optional(),
  agentSessionId: z.string().optional(),
})

// ============================================
// Specific Event Schemas
// ============================================

// Task event
export const TaskEventSchema = BaseEventSchema.extend({
  type: z.literal('task'),
  subType: z.enum(['created', 'open', 'closed', 'updated']),
  agentType: z.string(),
  payload: TaskPayloadSchema,
})

// Task iteration event
export const TaskIterationEventSchema = BaseEventSchema.extend({
  type: z.literal('taskIteration'),
  subType: z.enum(['created', 'active', 'completed', 'error', 'updated']),
  agentType: z.string(),
  iterationId: z.number(),
  payload: TaskIterationPayloadSchema,
})

// Workspace event
export const WorkspaceEventSchema = BaseEventSchema.extend({
  type: z.literal('workspace'),
  subType: z.enum(['ready', 'destroyed', 'error']),
  payload: EventPayloadSchema,
})

// Environment event
export const EnvironmentEventSchema = BaseEventSchema.extend({
  type: z.literal('environment'),
  subType: EnvironmentStatusSchema,
  payload: EventPayloadSchema,
})

// Server event
export const ServerEventSchema = BaseEventSchema.extend({
  type: z.literal('server'),
  subType: z.enum(['installing', 'starting', 'started', 'stopped', 'failed', 'error']),
  payload: ServerEventPayloadSchema,
})

// Navigation event
export const NavigationEventSchema = BaseEventSchema.extend({
  type: z.literal('navigation'),
  subType: z.enum(['navigated', 'loaded', 'refreshed']),
  payload: NavigationEventPayloadSchema,
})

// Preview event
export const PreviewEventSchema = BaseEventSchema.extend({
  type: z.literal('preview'),
  subType: z.enum(['playwright-action', 'mode-change', 'context-dirty']),
  payload: PreviewEventPayloadSchema,
})

// Agent event (consolidated: messages, results, lifecycle)
export const AgentEventSchema = BaseEventSchema.extend({
  type: z.literal('agent'),
  subType: z.enum(['started', 'stopped', 'message', 'completed', 'error']),
  agentType: z.string(),
  iterationId: z.number().optional(),
  payload: z.union([
    ClaudeCodeSDKMessageSchema.and(AgentEventPayloadSchema), // for 'message' subtype
    SDKResultMessageSchema.and(AgentEventPayloadSchema), // for 'completed' subtype
    EventPayloadSchema.and(AgentEventPayloadSchema), // for other subtypes
  ]),
})

// Event is a discriminated union of all specific event types
export const EventSchema = z.discriminatedUnion('type', [
  TaskEventSchema,
  TaskIterationEventSchema,
  WorkspaceEventSchema,
  EnvironmentEventSchema,
  ServerEventSchema,
  NavigationEventSchema,
  PreviewEventSchema,
  AgentEventSchema,
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
// Derived Types
// ============================================

export type EventType = z.infer<typeof EventTypeSchema>
export type BaseEvent = z.infer<typeof BaseEventSchema>
export type EventPayload = z.infer<typeof EventPayloadSchema>

// Re-export types from claude.js
export type {
  ClaudeCodeSDKMessage,
  SDKResultMessage,
} from '../types/claude.js'

export type TaskEvent = z.infer<typeof TaskEventSchema>
export type TaskIterationEvent = z.infer<typeof TaskIterationEventSchema>
export type WorkspaceEvent = z.infer<typeof WorkspaceEventSchema>
export type EnvironmentEvent = z.infer<typeof EnvironmentEventSchema>
export type ServerEvent = z.infer<typeof ServerEventSchema>
export type NavigationEvent = z.infer<typeof NavigationEventSchema>
export type PreviewEvent = z.infer<typeof PreviewEventSchema>
export type AgentEvent = z.infer<typeof AgentEventSchema>

// Event is the discriminated union of all specific event types
export type Event = z.infer<typeof EventSchema>

export type EventStreamMessage = z.infer<typeof EventStreamMessageSchema>
