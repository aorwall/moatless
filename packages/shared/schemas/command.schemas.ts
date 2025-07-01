import { z } from 'zod'

// Command name validation: alphanumeric, hyphens, and underscores
export const CommandNameSchema = z
  .string()
  .min(1)
  .max(100)
  .regex(
    /^[a-zA-Z0-9_-]+$/,
    'Name must contain only alphanumeric characters, hyphens, and underscores',
  )

// Command schemas
export const CommandSchema = z.object({
  name: CommandNameSchema,
  description: z.string(),
  prompt: z.string(),
})

export const CreateCommandSchema = z.object({
  name: CommandNameSchema,
  description: z.string(),
  prompt: z.string(),
})

export const UpdateCommandSchema = z.object({
  name: CommandNameSchema.optional(),
  description: z.string().optional(),
  prompt: z.string().optional(),
})

// Parameter schemas
export const CommandNameParamsSchema = z.object({
  commandName: CommandNameSchema,
})

// Type exports
export type Command = z.infer<typeof CommandSchema>
export type CreateCommandRequest = z.infer<typeof CreateCommandSchema>
export type UpdateCommandRequest = z.infer<typeof UpdateCommandSchema>
