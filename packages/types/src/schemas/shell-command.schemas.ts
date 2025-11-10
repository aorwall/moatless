import { z } from 'zod'

// ============================================
// Shell Command Core Schemas
// ============================================

export const ShellCommandStatusSchema = z.enum(['idle', 'running', 'completed', 'failed', 'killed'])

z.object({
  workingDirectory: z.string().optional(),
  env: z.record(z.string(), z.string()).optional(),
  shell: z.string().optional().describe('Shell to use (bash, zsh, sh)'),
  timeout: z.number().optional().describe('Timeout in milliseconds'),
  logFile: z.string().optional().describe('Path to persist logs'),
})

z.object({
  command: z.string(),
  status: ShellCommandStatusSchema,
  pid: z.number().optional(),
  startTime: z.string().optional(),
  endTime: z.string().optional(),
  exitCode: z.number().optional(),
  workingDirectory: z.string(),
  shell: z.string(),
  error: z.string().optional(),
  duration: z.number().optional().describe('Duration in milliseconds'),
})

export const ShellCommandLogEventSchema = z.object({
  type: z.enum(['stdout', 'stderr', 'system']),
  timestamp: z.string(),
  text: z.string(),
})

// ============================================
// Shell Command Result Schema
// ============================================

export const ShellCommandResultSchema = z.object({
  command: z.string(),
  output: z.string(),
  exitCode: z.number(),
  durationMs: z.number(),
  success: z.boolean().describe('True if exitCode is 0'),
  error: z.string().optional().describe('Error message if command failed'),
})

// ============================================
// Export Types
// ============================================

export type ShellCommandLogEvent = z.infer<typeof ShellCommandLogEventSchema>
