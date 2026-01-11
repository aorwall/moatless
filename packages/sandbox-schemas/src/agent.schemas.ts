import { z } from 'zod'

/**
 * File attachment schema for agent execution
 */
export const FileAttachmentSchema = z.object({
  fileId: z.string(),
  filename: z.string(),
  mimeType: z.string(),
  data: z.string(), // base64 encoded
})

export type FileAttachment = z.infer<typeof FileAttachmentSchema>

/**
 * Agent run options schema
 */
export const AgentRunOptionsSchema = z.object({
  workspaceDir: z.string().optional(),
  model: z.string().optional(),
  systemPrompt: z.string().optional(),
  maxTokens: z.number().optional(),
  fileAttachments: z.array(FileAttachmentSchema).optional(),
})

export type AgentRunOptions = z.infer<typeof AgentRunOptionsSchema>

/**
 * Execute agent request schema
 */
export const ExecuteAgentRequestSchema = z.object({
  taskId: z.string(),
  message: z.string(),
  agentType: z.string().optional().default('claude-code'),
  runOptions: AgentRunOptionsSchema.optional(),
  model: z.string().optional(),
})

export type ExecuteAgentRequest = z.infer<typeof ExecuteAgentRequestSchema>

/**
 * Execute agent response schema
 */
export const ExecuteAgentResponseSchema = z.object({
  success: z.boolean(),
  message: z.string().optional(),
  error: z.string().optional(),
})

export type ExecuteAgentResponse = z.infer<typeof ExecuteAgentResponseSchema>

/**
 * Stop agent response schema
 */
export const StopAgentResponseSchema = z.object({
  success: z.boolean(),
  agentStopped: z.boolean().optional(),
  error: z.string().optional(),
})

export type StopAgentResponse = z.infer<typeof StopAgentResponseSchema>

/**
 * Agent state enum
 */
export const AgentStateSchema = z.enum(['idle', 'running', 'completed'])

export type AgentState = z.infer<typeof AgentStateSchema>

/**
 * Agent status response schema
 */
export const AgentStatusResponseSchema = z.object({
  state: AgentStateSchema,
})

export type AgentStatusResponse = z.infer<typeof AgentStatusResponseSchema>
