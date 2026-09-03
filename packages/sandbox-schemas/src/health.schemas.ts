import { z } from 'zod'

export const HealthResponseSchema = z.object({
  status: z.literal('ok'),
  mode: z.string().optional(),
  workspaceDir: z.string().optional(),
  taskId: z.string().optional(),
  version: z.string().optional(),
  uptime: z.number().optional(),
})

export type HealthResponse = z.infer<typeof HealthResponseSchema>
