import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

const BaseMessageSchema = z.object({
  id: z.string(),
  role: z.enum(['user', 'assistant']),
  timestamp: dateSchema,
  eventId: z.string().optional(),
})

// Context schemas for providing additional context to the agent
const ElementContextDataSchema = z.object({
  sourceId: z.string().nullable(),
  componentName: z.string().nullable(),
  path: z.string(),
  lineNumber: z.number(),
  file: z.string(),
  elementType: z.string().nullable(),
  content: z.string().nullable(),
  className: z.string().nullable(),
  parents: z
    .array(
      z.object({
        sourceId: z.string().nullable(),
        componentName: z.string().nullable(),
        path: z.string(),
        lineNumber: z.number(),
        file: z.string(),
        elementType: z.string().nullable(),
        content: z.string().nullable(),
        className: z.string().nullable(),
      }),
    )
    .optional(),
})

const FigmaContextDataSchema = z.object({
  importId: z.string().uuid(),
  fileName: z.string(),
  nodeName: z.string(),
})

const MessageContextSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('element'),
    data: ElementContextDataSchema,
  }),
  z.object({
    type: z.literal('figma'),
    data: FigmaContextDataSchema,
  }),
  // Future context types can be added here:
  // z.object({ type: z.literal('file'), data: FileContextDataSchema }),
  // z.object({ type: z.literal('code-snippet'), data: CodeSnippetContextDataSchema }),
])

const AttachmentSchema = z.object({
  fileId: z.string(),
  filename: z.string(),
  mimeType: z.string(),
})

const UserMessageSchema = BaseMessageSchema.extend({
  role: z.literal('user'),
  content: z.string(),
  attachments: z.array(AttachmentSchema).optional(),
  contexts: z.array(MessageContextSchema).optional(),
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

// Export AttachmentSchema and MessageContextSchema for reuse in other schemas
export { AttachmentSchema, MessageContextSchema }

export type MessagesResponse = z.infer<typeof MessagesResponseSchema>
export type Message = z.infer<typeof MessageSchema>
export type UserMessage = z.infer<typeof UserMessageSchema>
export type AssistantMessage = z.infer<typeof AssistantMessageSchema>
export type AssistantMessageContent = z.infer<typeof AssistantMessageContentSchema>
export type ToolResult = z.infer<typeof ToolResultSchema>
export type Attachment = z.infer<typeof AttachmentSchema>
export type MessageContext = z.infer<typeof MessageContextSchema>
export type ElementContextData = z.infer<typeof ElementContextDataSchema>
export type FigmaContextData = z.infer<typeof FigmaContextDataSchema>
