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

// Command condition schemas
export const CommandConditionsSchema = z
  .object({
    // Git status conditions
    requiresUncommittedChanges: z.boolean().optional(),
    requiresCommittedChanges: z.boolean().optional(),
    requiresBehindMain: z.boolean().optional(),
    requiresAheadOfMain: z.boolean().optional(),
    requiresDiverged: z.boolean().optional(),

    // Task status conditions
    requiresTaskNotRunning: z.boolean().optional(),
    requiresLastIterationError: z.boolean().optional(),
    requiresLastIterationSuccess: z.boolean().optional(),

    // Git sync status values
    allowedGitSyncStatuses: z
      .array(z.enum(['synced', 'ahead', 'behind', 'diverged', 'uncommitted_changes', 'error']))
      .optional(),

    // Task status values
    allowedTaskStatuses: z
      .array(z.enum(['open', 'running', 'error', 'merged', 'closed']))
      .optional(),
  })
  .optional()

// Command schemas
export const CommandSchema = z.object({
  name: CommandNameSchema,
  description: z.string(),
  prompt: z.string(),
  conditions: CommandConditionsSchema,
  // UI customization
  icon: z.string().optional(), // Icon name like 'GitCommit', 'GitBranch', etc.
  variant: z.enum(['default', 'outline', 'secondary', 'ghost', 'destructive']).optional(),
  active: z.boolean().optional(), // Whether command is active/visible (defaults to true)
  willNotResume: z.boolean().optional(), // Whether this command creates iterations that won't be resumed
})

export const CreateCommandSchema = z.object({
  name: CommandNameSchema,
  description: z.string(),
  prompt: z.string(),
  conditions: CommandConditionsSchema,
  icon: z.string().optional(),
  variant: z.enum(['default', 'outline', 'secondary', 'ghost', 'destructive']).optional(),
  active: z.boolean().optional(),
  willNotResume: z.boolean().optional(),
})

export const UpdateCommandSchema = z.object({
  name: CommandNameSchema.optional(),
  description: z.string().optional(),
  prompt: z.string().optional(),
  conditions: CommandConditionsSchema,
  icon: z.string().optional(),
  variant: z.enum(['default', 'outline', 'secondary', 'ghost', 'destructive']).optional(),
  active: z.boolean().optional(),
  willNotResume: z.boolean().optional(),
})

// Parameter schemas
export const CommandNameParamsSchema = z.object({
  commandName: CommandNameSchema,
})

// Type exports
export type Command = z.infer<typeof CommandSchema>
export type CreateCommandRequest = z.infer<typeof CreateCommandSchema>
export type UpdateCommandRequest = z.infer<typeof UpdateCommandSchema>
