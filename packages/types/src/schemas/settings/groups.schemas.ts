import { z } from 'zod'

/**
 * Group configuration schema
 * Maps user IDs to group IDs for group-based access control
 *
 * Example:
 * {
 *   userGroups: {
 *     "12345": "team-frontend",
 *     "67890": "team-frontend",
 *     "11111": "team-backend"
 *   }
 * }
 */
export const GroupConfigSchema = z.object({
  // Map of userId (as string) to groupId
  userGroups: z.record(z.string(), z.string()),
})

export type GroupConfig = z.infer<typeof GroupConfigSchema>
