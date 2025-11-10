import { z } from 'zod'

export const DeployRequestSchema = z.object({
  // No parameters needed - everything comes from task/settings
})

export const DeployResponseSchema = z.object({
  success: z.boolean(),
  siteId: z.string().optional(),
  deployUrl: z.string().optional(),
  siteUrl: z.string().optional(),
  siteName: z.string().optional(),
  deployId: z.string().optional(),
  buildDuration: z.number().optional(),
  deployDuration: z.number().optional(),
  error: z.string().optional(),
  logs: z.string().optional(),
})

// Type exports
export type DeployResponse = z.infer<typeof DeployResponseSchema>
