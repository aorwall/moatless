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
  url: z.string().optional(),
  pid: z.number().optional(),
  uptimeSeconds: z.number().optional(),
  error: z.string().optional(),
  logs: z.string().optional(),
  currentPath: z.string().optional(),
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
export type CommandExecution = z.infer<typeof CommandExecutionSchema>
export type ServerStatus = z.infer<typeof ServerStatusSchema>
export type ServerStopResponse = z.infer<typeof ServerStopResponseSchema>
export type ServerLogsQuery = z.infer<typeof ServerLogsQuerySchema>
export type ServerLogsResponseExtended = z.infer<typeof ServerLogsResponseExtendedSchema>
