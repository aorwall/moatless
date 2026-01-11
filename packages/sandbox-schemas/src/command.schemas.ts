import { z } from 'zod'

/**
 * Command execution request schema
 */
export const CommandRequestSchema = z.object({
  command: z.string().min(1),
  timeout: z.number().optional(),
  env: z.record(z.string(), z.string()).optional(),
})

export type CommandRequest = z.infer<typeof CommandRequestSchema>

/**
 * Command execution response schema
 */
export const CommandResponseSchema = z.object({
  exit_code: z.number(),
  stdout: z.string(),
  stderr: z.string(),
})

export type CommandResponse = z.infer<typeof CommandResponseSchema>

/**
 * Shell command log event schema (for streaming)
 */
export const ShellCommandLogEventSchema = z.object({
  type: z.enum(['stdout', 'stderr', 'system']),
  timestamp: z.string(),
  text: z.string(),
})

export type ShellCommandLogEvent = z.infer<typeof ShellCommandLogEventSchema>
