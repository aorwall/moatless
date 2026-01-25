import { z } from 'zod'
import { AttachmentSchema, MessageContextSchema } from './message.schemas.js'

// Tool use result schema for tool execution results
const ToolUseResultSchema = z.object({
  stdout: z.string(),
  stderr: z.string(),
  interrupted: z.boolean(),
  isImage: z.boolean(),
})

// Base schema for all agent messages
const BaseAgentMessageSchema = z.object({
  parentUuid: z.string().optional(),
  isSidechain: z.boolean().optional(),
  userType: z.string(),
  cwd: z.string().optional(),
  sessionId: z.string().optional(),
  iterationNumber: z.number().optional(),
  version: z.string().optional(),
  gitBranch: z.string().optional(),
  uuid: z.string(),
  timestamp: z.string(), // ISO date string
  isVisibleInTranscriptOnly: z.boolean().optional(),
  isCompactSummary: z.boolean().optional(),
  // Agent session ID - set when agent acknowledges receipt of this message
  // Null means message hasn't been received by agent yet (pending acknowledgment)
  agentSessionId: z.string().optional(),
})

// User agent message schema
const UserAgentMessageSchema = BaseAgentMessageSchema.extend({
  type: z.literal('user'),
  message: z.object({
    role: z.literal('user'),
    content: z.union([
      z.string(),
      z.array(
        z.union([
          z.object({
            type: z.literal('text'),
            text: z.string(),
          }),
          z.object({
            type: z.literal('tool_result'),
            tool_use_id: z.string(),
            content: z.string(),
            is_error: z.boolean().optional(),
          }),
        ]),
      ),
    ]),
  }),
  toolUseResult: ToolUseResultSchema.optional(),
  attachments: z.array(AttachmentSchema).optional(),
  contexts: z.array(MessageContextSchema).optional(),
})

// System agent message schema (SDK user messages that are tool results)
const SystemAgentMessageSchema = BaseAgentMessageSchema.extend({
  type: z.literal('system'),
  message: z.object({
    role: z.literal('user'),
    content: z.union([
      z.string(),
      z.array(
        z.union([
          z.object({
            type: z.literal('text'),
            text: z.string(),
          }),
          z.object({
            type: z.literal('tool_result'),
            tool_use_id: z.string(),
            content: z.string(),
            is_error: z.boolean().optional(),
          }),
        ]),
      ),
    ]),
  }),
  toolUseResult: ToolUseResultSchema.optional(),
})

// Assistant agent message schema
const AssistantAgentMessageSchema = BaseAgentMessageSchema.extend({
  type: z.literal('assistant'),
  message: z.object({
    id: z.string().optional(),
    type: z.literal('message'),
    role: z.literal('assistant'),
    model: z.string().optional(),
    stop_reason: z.string().nullable().optional(),
    stop_sequence: z.string().nullable().optional(),
    usage: z
      .object({
        input_tokens: z.number(),
        output_tokens: z.number(),
        cache_creation_input_tokens: z.number().optional(),
        cache_read_input_tokens: z.number().optional(),
        server_tool_use: z
          .object({
            web_search_requests: z.number(),
          })
          .optional(),
        service_tier: z.string().nullable().optional(),
      })
      .optional(),
    content: z.array(
      z.union([
        z.object({
          type: z.literal('text'),
          text: z.string(),
        }),
        z.object({
          type: z.literal('tool_use'),
          id: z.string(),
          name: z.string(),
          input: z.record(z.string(), z.unknown()),
        }),
      ]),
    ),
  }),
  requestId: z.string().optional(),
})

// Discriminated union of agent messages
export const AgentMessageSchema = z.union([
  UserAgentMessageSchema,
  AssistantAgentMessageSchema,
  SystemAgentMessageSchema,
])

// Response schema for agent messages
export const AgentMessagesResponseSchema = z.object({
  messages: z.array(AgentMessageSchema),
  totalCount: z.number(),
})

// Export TypeScript types
export type AgentMessage = z.infer<typeof AgentMessageSchema>
export type UserAgentMessage = z.infer<typeof UserAgentMessageSchema>
export type AssistantAgentMessage = z.infer<typeof AssistantAgentMessageSchema>
export type SystemAgentMessage = z.infer<typeof SystemAgentMessageSchema>
export type ToolUseResult = z.infer<typeof ToolUseResultSchema>
export type AgentMessagesResponse = z.infer<typeof AgentMessagesResponseSchema>
