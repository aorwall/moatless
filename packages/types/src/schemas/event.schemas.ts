import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'
import { EnvironmentStatusSchema } from './workspace.schemas.js'

// ============================================
// Event Types and Enums
// ============================================

// Event type enum using string literals (following schema design patterns)
// Consolidated from 12 types to 6 logical types
export const EventTypeSchema = z.enum([
  'task', // Task lifecycle
  'taskTurn', // Task turn execution
  'workspace', // Workspace operations
  'environment', // Environment setup/status, TODO: Replace this with workspace event  and replace workspace status with the status set on environment
  'agent', // Agent lifecycle + messages + results
  'server', // Preview server management
  'navigation', // Preview navigation events, TODO: Investigate if we use / can use this
  'preview', // Playwright preview interaction events, TODO: Investigate if we use / can use this
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

// Task turn payload schema
export const TaskTurnPayloadSchema = z.object({
  status: z.string(),
  previousStatus: z.string(),
  turnNumber: z.number(),
})

// Server event payload schema
export const ServerEventPayloadSchema = z.object({
  serverName: z.string(),
  status: z.enum(['stopped', 'installing', 'starting', 'started', 'failed']),
  previousStatus: z.enum(['stopped', 'installing', 'starting', 'started', 'failed']).optional(),
  error: z.string().optional(),
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
  agentType: z.string(), // TODO: Remove
  payload: TaskPayloadSchema,
})

// Task turn event
export const TaskTurnEventSchema = BaseEventSchema.extend({
  type: z.literal('taskTurn'),
  subType: z.enum(['active', 'completed', 'error']),
  agentType: z.string(),
  turnNumber: z.number(),
  payload: TaskTurnPayloadSchema,
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

// Pending response payload schema for interactive tools (AskUserQuestion, ExitPlanMode)
export const PendingResponsePayloadSchema = z.object({
  toolName: z.enum(['AskUserQuestion', 'ExitPlanMode']),
  toolInput: z.record(z.string(), z.unknown()),
  // For AskUserQuestion: contains questions array
  // For ExitPlanMode: may contain plan content
})

// Agent event (consolidated: messages, results, lifecycle)
export const AgentEventSchema = BaseEventSchema.extend({
  type: z.literal('agent'),
  subType: z.enum(['started', 'stopped', 'message', 'completed', 'error', 'pending_response']),
  agentType: z.string(),
  turnNumber: z.number().optional(),
  payload: z.union([
    ClaudeCodeSDKMessageSchema.and(AgentEventPayloadSchema), // for 'message' subtype
    SDKResultMessageSchema.and(AgentEventPayloadSchema), // for 'completed' subtype
    PendingResponsePayloadSchema.and(AgentEventPayloadSchema), // for 'pending_response' subtype
    EventPayloadSchema.and(AgentEventPayloadSchema), // for other subtypes
  ]),
})

// Event is a discriminated union of all specific event types
export const EventSchema = z.discriminatedUnion('type', [
  TaskEventSchema,
  TaskTurnEventSchema,
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
z.object({
  type: z.string(),
  timestamp: z.string(),
  data: z.any().optional(), // Event-specific data
})

// Stream Status Response Schema (moved from api.schemas.ts)
z.object({
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

// Re-export types from claude.js
export type {
  ClaudeCodeSDKMessage,
  SDKResultMessage,
} from '../types/claude.js'

export type TaskEvent = z.infer<typeof TaskEventSchema>
export type TaskTurnEvent = z.infer<typeof TaskTurnEventSchema>
export type WorkspaceEvent = z.infer<typeof WorkspaceEventSchema>
export type ServerEvent = z.infer<typeof ServerEventSchema>
export type AgentEvent = z.infer<typeof AgentEventSchema>
export type PendingResponsePayload = z.infer<typeof PendingResponsePayloadSchema>

// Event is the discriminated union of all specific event types
export type Event = z.infer<typeof EventSchema>
