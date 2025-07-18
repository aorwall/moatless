import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

const BaseMessageSchema = z.object({
  id: z.string(),
  role: z.enum(['user', 'assistant']),
  timestamp: dateSchema,
  eventId: z.string().optional(),
})

const UserMessageSchema = BaseMessageSchema.extend({
  role: z.literal('user'),
  content: z.string(),
})

const ToolResultSchema = z.object({
  content: z.string(),
  is_error: z.boolean().optional(),
  eventId: z.string().optional(),
})

const AssistantMessageContentSchema = z.object({
  type: z.enum(['text', 'tool_use']),
  text: z.string().optional(),
  tool_name: z.string().optional(),
  tool_input: z.record(z.string(), z.unknown()).optional(),
  tool_use_id: z.string().optional(),
  tool_result: ToolResultSchema.optional(),
})

const AssistantMessageSchema = BaseMessageSchema.extend({
  role: z.literal('assistant'),
  content: z.union([z.string(), z.array(AssistantMessageContentSchema)]),
})

export const MessageSchema = z.union([UserMessageSchema, AssistantMessageSchema])

export const MessagesResponseSchema = z.object({
  messages: z.array(MessageSchema),
  totalCount: z.number(),
})

export type MessagesResponse = z.infer<typeof MessagesResponseSchema>
export type Message = z.infer<typeof MessageSchema>
export type UserMessage = z.infer<typeof UserMessageSchema>
export type AssistantMessage = z.infer<typeof AssistantMessageSchema>
export type AssistantMessageContent = z.infer<typeof AssistantMessageContentSchema>
export type ToolResult = z.infer<typeof ToolResultSchema>
