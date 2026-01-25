import { MessageContextSchema, UserContentSchema } from '@moatless/types'
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
 * for HTTP transport. Apps should extend this type locally to add `onEvent`.
 */
export const AgentRunRequestSchema = z.object({
  // Content to send to agent (primary way to send messages/content)
  content: z.array(UserContentSchema).optional(),

  // Execution environment
  workspaceDir: z.string().optional(),
  environmentVariables: z.record(z.string(), z.string()).optional(),
  timeout: z.number().optional(),

  // Agent control
  iterationNumber: z.number().optional(),
  resume: z.string().optional(),
  model: z.string().optional(),
  agentMode: z.enum(['edit', 'plan']).optional(),
  mcpServerUrl: z.string().optional(),
  hasPreviewServer: z.boolean().optional(),
  appendSystemPrompt: z.string().optional(),

  // Background mode: bypass permissions, deny interactive tools, cleanup workspace after completion
  background: z.boolean().optional(),

  // Plugin paths to load (resolved by backend, absolute paths in shared storage)
  pluginPaths: z.array(z.string()).optional(),

  // File attachments with actual data for transfer to sandbox pods (K8s mode)
  fileAttachments: z.array(FileAttachmentSchema).optional(),

  // --- Deprecated fields (kept for backward compatibility during migration) ---
  // @deprecated Use content field instead
  fileIds: z.array(z.string()).optional(),
  // @deprecated Use content field instead
  contexts: z.array(MessageContextSchema).optional(),
  // @deprecated Use content field instead
  contentBlocks: z.array(UserContentSchema).optional(),
})

export type AgentRunRequest = z.infer<typeof AgentRunRequestSchema>

/**
 * @deprecated Use AgentRunRequestSchema and AgentRunRequest instead
 */
export const AgentRunOptionsSchema = AgentRunRequestSchema
export type AgentRunOptions = AgentRunRequest

/**
 * Execute agent request schema
 */
export const ExecuteAgentRequestSchema = z.object({
  taskId: z.string(),
  message: z.string(),
  agentType: z.string().optional().default('claude-code'),
  runOptions: AgentRunRequestSchema.optional(),
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
