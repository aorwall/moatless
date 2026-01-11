import type { Event, MessageContext, UserContent } from '@moatless/types'

export interface AgentOptions {
  model?: string
  apiKey?: string
  maxTokens?: number
  temperature?: number
  timeout?: number
  [key: string]: unknown
}

/**
 * File attachment with base64-encoded content for transfer to worker pods.
 * Used in Kubernetes mode where the worker doesn't have access to the main API's filesystem.
 */
export interface FileAttachment {
  fileId: string
  filename: string
  mimeType: string
  data: string // base64 encoded file content
}

export interface AgentRunOptions {
  workspaceDir?: string
  environmentVariables?: Record<string, string>
  timeout?: number
  onProgress?: (event: Event) => void
  iterationNumber?: number
  resume?: string
  model?: string
  agentMode?: 'edit' | 'plan'
  mcpServerUrl?: string
  hasPreviewServer?: boolean
  appendSystemPrompt?: string
  // User message metadata for message persistence
  fileIds?: string[]
  contexts?: MessageContext[]
  // File attachments to send as content blocks (images, text files, etc.)
  contentBlocks?: UserContent[]
  // File attachments with actual data for transfer to worker pods (K8s mode)
  fileAttachments?: FileAttachment[]
  // Flag to skip user message save in startAgent (already saved by caller)
  userMessageSaved?: boolean
}

export abstract class BaseAgent {
  protected stopRequested = false
  protected iterationNumber: number | undefined
  protected sessionId: string | undefined

  protected constructor(
    protected taskId: string,
    protected options: AgentOptions = {},
  ) {}

  abstract run(message: string, options?: AgentRunOptions): AsyncGenerator<Event>
  abstract stop(): Promise<void>
  abstract getAgentType(): string

  protected delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
