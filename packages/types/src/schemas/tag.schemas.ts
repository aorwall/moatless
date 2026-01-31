import { z } from 'zod'

/**
 * Tag schema for task categorization.
 * Name must be URL-safe: lowercase alphanumeric and dashes only.
 */
export const TagSchema = z.object({
  tagId: z.string(),
  name: z
    .string()
    .regex(/^[a-z0-9-]+$/, 'Tag name must be URL-safe (lowercase alphanumeric and dashes only)'),
  description: z.string().optional(),
})

export const TagResponseSchema = TagSchema

export const CreateTagRequestSchema = z.object({
  name: z
    .string()
    .regex(/^[a-z0-9-]+$/, 'Tag name must be URL-safe (lowercase alphanumeric and dashes only)'),
  description: z.string().optional(),
})

export type Tag = z.infer<typeof TagSchema>
export type CreateTagRequest = z.infer<typeof CreateTagRequestSchema>
