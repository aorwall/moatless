/**
 * Zod schemas for agent content types
 * These correspond to the TypeScript interfaces in claude-content.ts
 *
 * Note: Type aliases are not exported from this file to avoid conflicts
 * with the existing TypeScript interfaces in claude-content.ts.
 * Use the types from claude-content.ts or use z.infer<typeof Schema> if needed.
 */
import { z } from 'zod'

/**
 * Text content schema for user messages
 */
export const TextContentSchema = z.object({
  type: z.literal('text'),
  text: z.string(),
})

/**
 * Tool result content schema for returning tool execution results
 */
export const ToolResultContentSchema = z.object({
  type: z.literal('tool_result'),
  tool_use_id: z.string(),
  content: z.string(),
  is_error: z.boolean().optional(),
})

/**
 * Image content schema for base64-encoded images
 */
export const ImageContentBlockSchema = z.object({
  type: z.literal('image'),
  source: z.object({
    type: z.literal('base64'),
    media_type: z.enum(['image/png', 'image/jpeg', 'image/gif', 'image/webp']),
    data: z.string(),
  }),
})

/**
 * Union schema for all user content types that can be sent to an agent
 */
export const UserContentSchema = z.discriminatedUnion('type', [
  TextContentSchema,
  ToolResultContentSchema,
  ImageContentBlockSchema,
])
