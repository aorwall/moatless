import { z } from 'zod'
import { dateSchema } from '../utils/validation.js'

const BaseMessageSchema = z.object({
  id: z.string(),
  role: z.enum(['user', 'assistant']),
  timestamp: dateSchema,
  eventId: z.string().optional(),
  isVisibleInTranscriptOnly: z.boolean().optional(),
  isCompactSummary: z.boolean().optional(),
})

// Context schemas for providing additional context to the agent
const ElementContextDataSchema = z.object({
  componentId: z.string(), // "path:line:column" format
  path: z.string(),
  line: z.number(),
  column: z.number(),
  componentName: z.string(),
})

const FigmaContextDataSchema = z.object({
  importId: z.uuid(),
  fileName: z.string(),
  nodeName: z.string(),
  hasImage: z.boolean(),
})

const PreviewUrlContextDataSchema = z.object({
  path: z.string(), // The path being viewed (e.g., "/about")
  url: z.string().optional(), // Optional full URL if needed
})

const MessageContextSchema = z.discriminatedUnion('type', [
  z
    .object({
      type: z.literal('element'),
      referenceId: z.string(),
      data: ElementContextDataSchema,
    })
    .refine((ctx) => ctx.referenceId === ctx.data.componentId, {
      message: 'referenceId must match componentId for element contexts',
      path: ['referenceId'],
    }),
  z
    .object({
      type: z.literal('figma'),
      referenceId: z.string(),
      data: FigmaContextDataSchema,
    })
    .refine((ctx) => ctx.referenceId === ctx.data.importId, {
      message: 'referenceId must match importId for figma contexts',
      path: ['referenceId'],
    }),
  z.object({
    type: z.literal('previewUrl'),
    referenceId: z.string(), // The path serves as referenceId
    data: PreviewUrlContextDataSchema,
  }),
  // Future context types can be added here:
  // z.object({ type: z.literal('file'), referenceId: z.string(), data: FileContextDataSchema }),
  // z.object({ type: z.literal('code-snippet'), referenceId: z.string(), data: CodeSnippetContextDataSchema }),
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

export type Message = z.infer<typeof MessageSchema>
export type UserMessage = z.infer<typeof UserMessageSchema>
export type AssistantMessage = z.infer<typeof AssistantMessageSchema>
export type AssistantMessageContent = z.infer<typeof AssistantMessageContentSchema>
export type ToolResult = z.infer<typeof ToolResultSchema>
export type Attachment = z.infer<typeof AttachmentSchema>
export type MessageContext = z.infer<typeof MessageContextSchema>
export type ElementContextData = z.infer<typeof ElementContextDataSchema>
export type PreviewUrlContextData = z.infer<typeof PreviewUrlContextDataSchema>
