import { z } from 'zod'

/**
 * Preview server status enum
 */
export const ServerStatusSchema = z.enum(['stopped', 'installing', 'starting', 'started', 'failed'])

export type ServerStatus = z.infer<typeof ServerStatusSchema>

/**
 * Start server request schema
 */
export const StartServerRequestSchema = z.object({
  serverCommand: z.string().optional(),
  serverCommands: z.array(z.string()).optional(),
  envVars: z.record(z.string(), z.string()).optional(),
  setupCommands: z.array(z.string()).optional(),
  startupTimeoutSeconds: z.number().optional(),
})

export type StartServerRequest = z.infer<typeof StartServerRequestSchema>

/**
 * Start server response schema
 */
export const StartServerResponseSchema = z.object({
  success: z.boolean(),
  status: ServerStatusSchema.optional(),
  port: z.number().optional(),
  pid: z.number().optional(),
  url: z.string().optional(),
  uptimeSeconds: z.number().optional(),
  error: z.string().optional(),
})

export type StartServerResponse = z.infer<typeof StartServerResponseSchema>

/**
 * Stop server response schema
 */
export const StopServerResponseSchema = z.object({
  success: z.boolean(),
  message: z.string().optional(),
  error: z.string().optional(),
})

export type StopServerResponse = z.infer<typeof StopServerResponseSchema>

/**
 * Server status response schema
 */
export const ServerStatusResponseSchema = z.object({
  status: ServerStatusSchema,
  running: z.boolean().optional(),
  configured: z.boolean().optional(),
  internalUrl: z.string().optional(),
  externalUrl: z.string().optional(),
  port: z.number().optional(),
  pid: z.number().optional(),
  uptimeSeconds: z.number().optional(),
  error: z.string().optional(),
})

export type ServerStatusResponse = z.infer<typeof ServerStatusResponseSchema>

/**
 * Server logs response schema
 */
export const ServerLogsResponseSchema = z.object({
  logs: z.string(),
})

export type ServerLogsResponse = z.infer<typeof ServerLogsResponseSchema>

/**
 * Server log event schema (for streaming)
 */
export const ServerLogEventSchema = z.object({
  type: z.string(),
  data: z.object({
    text: z.string().optional(),
    error: z.string().optional(),
    timestamp: z.string(),
  }),
})

export type ServerLogEvent = z.infer<typeof ServerLogEventSchema>

/**
 * Proxy request schema
 */
export const ProxyRequestSchema = z.object({
  method: z.string(),
  path: z.string(),
  headers: z.record(z.string(), z.string()).optional(),
  body: z.string().optional(),
})

export type ProxyRequest = z.infer<typeof ProxyRequestSchema>
