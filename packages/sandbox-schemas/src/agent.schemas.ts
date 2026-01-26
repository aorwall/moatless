import { UserContentSchema } from '@moatless/types'
import { z } from 'zod'

/**
 * File attachment schema for agent execution.
 * Used in Kubernetes mode where the sandbox doesn't have access to the main API's filesystem.
 */
export const FileAttachmentSchema = z.object({
  fileId: z.string(),
  filename: z.string(),
  mimeType: z.string(),
  data: z.string(), // base64 encoded
})

export type FileAttachment = z.infer<typeof FileAttachmentSchema>

/**
 * Comprehensive agent run request schema.
 * Contains content to send to the agent along with execution options.
 *
 * Note: The `onEvent` callback is NOT included here as it cannot be serialized
 * for HTTP transport. Apps should add `onEvent` separately when needed.
 *
 * Note: `workspaceDir` is NOT included here - sandbox derives it from its own
 * context (fixed mounted path). This keeps the request serializable and
 * environment-agnostic.
 */
export const AgentRunRequestSchema = z.object({
  // Content to send to agent (required - message text goes here)
  content: z.array(UserContentSchema),

  // Execution options
  timeout: z.number().optional(),
  // Resume session: string=specific session ID, true=use last session ID, false/undefined=new session
  resume: z.union([z.boolean(), z.string()]).optional(),
  model: z.string().optional(),
  agentMode: z.enum(['edit', 'plan']).optional(),
  appendSystemPrompt: z.string().optional(),

  // Background mode: bypass permissions, deny interactive tools, cleanup workspace after completion
  background: z.boolean().optional(),

  // Plugins - identifiers like "plugin-name@marketplace-name" (resolved to paths in sandbox)
  plugins: z.array(z.string()).optional(),

  // File attachments with actual data for transfer to sandbox pods (K8s mode)
  fileAttachments: z.array(FileAttachmentSchema).optional(),
})

export type AgentRunRequest = z.infer<typeof AgentRunRequestSchema>

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
