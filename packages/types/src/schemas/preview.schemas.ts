import { z } from 'zod'

export const ServerStatusEnumSchema = z.enum([
  'stopped',
  'installing',
  'starting',
  'started',
  'failed',
])

export const CommandExecutionSchema = z.object({
  command: z.string(),
  timeout: z.number().optional(),
})

export const ServerStatusSchema = z.object({
  status: ServerStatusEnumSchema,
  url: z.string().optional(), // External URL (proxy URL exposed to users)
  pid: z.number().optional(),
  uptimeSeconds: z.number().optional(),
  error: z.string().optional(),
  logs: z.string().optional(),
  currentPath: z.string().optional(),
  dirtyContext: z.boolean().optional(),
  // internalUrl is not exposed in the public API
})

export const ServerStopResponseSchema = z.object({
  stopped: z.boolean(),
  runtime: z.number(),
})

export const ServerLogsQuerySchema = z.object({
  lines: z.coerce.number().optional(),
})

export const ServerLogsResponseExtendedSchema = z.object({
  logs: z.string(),
  lines: z.number(),
  truncated: z.boolean(),
})

export type ServerStatusEnum = z.infer<typeof ServerStatusEnumSchema>
export type ServerStatus = z.infer<typeof ServerStatusSchema>
