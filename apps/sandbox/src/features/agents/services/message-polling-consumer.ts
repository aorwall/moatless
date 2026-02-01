import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { AgentMessage, ResultAgentMessage, UserAgentMessage } from '@moatless/types'
import type { SandboxConfig } from '../../../config/environment'
import type { BackendApiClient } from '../../../shared/backend-api-client'
import { createBackendApiClient } from '../../../shared/backend-api-client'
import { logger } from '../../../shared/logger'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'
import { createSandboxTools } from '../../tools'
import {
  getActivePromptToolName,
  hasActivePrompt,
  resolveInteractivePrompt,
} from '../claude-code/interactive-prompt'
import { AgentService } from './agent.service'

/**
 * Default poll timeout in milliseconds (how long to wait for notification before polling anyway)
 */
const POLL_TIMEOUT_MS = 5000

/**
 * Backoff delay on error
 */
const ERROR_BACKOFF_MS = 2000

/**
 * Convert a free-form message to the appropriate user input response format.
 * For AskUserQuestion: Uses the message as a freeform answer
 * For ExitPlanMode: Any message is treated as rejection with the message as feedback
 */
function convertMessageToUserInputResponse(
  toolName: 'AskUserQuestion' | 'ExitPlanMode' | null,
  messageText: string,
):
  | import('@moatless/types').AskUserQuestionResponse
  | import('@moatless/types').ExitPlanModeResponse {
  if (toolName === 'ExitPlanMode') {
    return {
      approved: false,
      feedback: messageText,
    }
  }

  // Default: AskUserQuestion - use message as freeform response
  return {
    answers: { _freeformResponse: messageText },
  }
}

/**
 * Response shape from the pending messages endpoint
 */
interface PendingMessagesResponse {
  messages: AgentMessage[]
  totalCount: number
}

/**
 * MessagePollingConsumer - Polls the backend API for pending user messages
 *
 * Replaces the Redis Streams-based AgentRequestConsumer with a simpler pattern:
 * - The sandbox polls the backend API for unsent user messages (agentSessionId IS NULL)
 * - The backend sends a POST /notify-messages to wake the sandbox when a new message arrives
 * - Messages are marked as sent (sentAt) when the agent acknowledges them via the existing event pipeline
 *
 * Features:
 * - Recovery on restart: immediately polls on startup
 * - Batching: all pending messages combined into a single agent call
 * - Interactive prompt handling (AskUserQuestion/ExitPlanMode)
 * - Notification-driven wake-up with 5s timeout fallback
 */
export class MessagePollingConsumer {
  private running = false
  private agentService: AgentService
  private backendApi: BackendApiClient | undefined
  private wakeUpResolver: (() => void) | null = null

  constructor(
    private readonly config: SandboxConfig,
    eventService: SandboxEventService | undefined,
  ) {
    const backendApi = createBackendApiClient(config)
    const tools = createSandboxTools(config, backendApi)
    this.backendApi = backendApi
    this.agentService = new AgentService(config.taskId, config.workspaceDir, eventService, tools)
  }

  /**
   * Called by the notification endpoint to wake up the poll loop immediately
   */
  notify(): void {
    if (this.wakeUpResolver) {
      this.wakeUpResolver()
      this.wakeUpResolver = null
    }
  }

  /**
   * Start the consumer loop
   */
  // biome-ignore lint/suspicious/useAwait: pollLoop runs in background intentionally
  async start(): Promise<void> {
    if (this.running) {
      logger.warn({ taskId: this.config.taskId }, 'MessagePollingConsumer: Already running')
      return
    }

    this.running = true

    logger.info({ taskId: this.config.taskId }, 'MessagePollingConsumer: Starting poll loop')

    this.pollLoop().catch((error) => {
      logger.error(
        { err: error, taskId: this.config.taskId },
        'MessagePollingConsumer: Poll loop crashed',
      )
    })
  }

  /**
   * Stop the consumer loop
   */
  stop(): void {
    this.running = false
    // Wake up the loop so it can exit
    this.notify()
    logger.info({ taskId: this.config.taskId }, 'MessagePollingConsumer: Stopping consumer')
  }

  /**
   * Get current agent state
   */
  getState(): { state: 'idle' | 'running'; hasAgent: boolean } {
    const serviceState = this.agentService.getState()
    return {
      state: serviceState.state,
      hasAgent: serviceState.hasAgent,
    }
  }

  /**
   * Stop current agent
   */
  async stopAgent(): Promise<void> {
    await this.agentService.stopAgent()
  }

  /**
   * Main poll loop - waits for notification or timeout, then fetches pending messages
   */
  private async pollLoop(): Promise<void> {
    while (this.running) {
      try {
        // Wait for notification or timeout
        await this.waitForWakeUp()

        if (!this.running) {
          break
        }

        // Fetch pending messages from backend
        const response = await this.fetchPendingMessages()

        if (!response) {
          continue
        }

        // Separate pending user messages from the last result message
        const userMessages = response.messages.filter(
          (m): m is UserAgentMessage => m.type === 'user',
        )
        const lastResult = response.messages.find(
          (m): m is ResultAgentMessage => m.type === 'result',
        )

        if (userMessages.length === 0) {
          continue
        }

        logger.info(
          {
            taskId: this.config.taskId,
            messageCount: userMessages.length,
            hasLastResult: !!lastResult,
          },
          'MessagePollingConsumer: Received pending messages',
        )

        // Execute the messages
        await this.executePendingMessages(userMessages, lastResult)
      } catch (error) {
        if (!this.running) {
          break
        }

        logger.error(
          { err: error, taskId: this.config.taskId },
          'MessagePollingConsumer: Error in poll loop, retrying',
        )

        await this.sleep(ERROR_BACKOFF_MS)
      }
    }

    logger.info({ taskId: this.config.taskId }, 'MessagePollingConsumer: Poll loop exited')
  }

  /**
   * Wait for either a notification signal or a timeout
   */
  private waitForWakeUp(): Promise<void> {
    return new Promise<void>((resolve) => {
      const timeout = setTimeout(() => {
        this.wakeUpResolver = null
        resolve()
      }, POLL_TIMEOUT_MS)

      this.wakeUpResolver = () => {
        clearTimeout(timeout)
        resolve()
      }
    })
  }

  /**
   * Fetch pending user messages from the backend API
   */
  private async fetchPendingMessages(): Promise<PendingMessagesResponse | null> {
    if (!this.backendApi) {
      logger.debug(
        { taskId: this.config.taskId },
        'MessagePollingConsumer: No backend API client, skipping fetch',
      )
      return null
    }

    try {
      const response = await this.backendApi.get(
        `/api/v1/tasks/${this.config.taskId}/messages/pending`,
      )

      if (!response.ok) {
        logger.warn(
          {
            taskId: this.config.taskId,
            status: response.status,
            statusText: response.statusText,
          },
          'MessagePollingConsumer: Failed to fetch pending messages',
        )
        return null
      }

      const data = (await response.json()) as PendingMessagesResponse
      return data
    } catch (error) {
      logger.error(
        { err: error, taskId: this.config.taskId },
        'MessagePollingConsumer: Error fetching pending messages',
      )
      return null
    }
  }

  /**
   * Execute pending user messages as an agent request
   */
  private async executePendingMessages(
    userMessages: UserAgentMessage[],
    lastResult: ResultAgentMessage | undefined,
  ): Promise<void> {
    const state = this.agentService.getState()

    if (state.state === 'running') {
      // Check if agent is waiting for user input
      if (hasActivePrompt()) {
        this.handleInteractivePrompt(userMessages)
        return
      }

      logger.warn(
        { taskId: this.config.taskId },
        'MessagePollingConsumer: Agent is already running, skipping',
      )
      return
    }

    // Convert pending messages to AgentRunRequest
    const request = this.convertToAgentRunRequest(userMessages, lastResult)

    if (userMessages.length === 1) {
      const result = await this.agentService.executeRequest(request)
      if (!result.success) {
        logger.error(
          { taskId: this.config.taskId, error: result.error },
          'MessagePollingConsumer: Agent execution failed',
        )
      }
    } else {
      const batchedRequest = this.agentService.createBatchedRequest(
        userMessages.map((msg) => this.convertSingleMessageToRequest(msg, lastResult)),
      )
      const result = await this.agentService.executeBatchedRequest(batchedRequest)
      if (!result.success) {
        logger.error(
          { taskId: this.config.taskId, error: result.error },
          'MessagePollingConsumer: Batched agent execution failed',
        )
      }
    }
  }

  /**
   * Handle interactive prompt resolution with incoming message
   */
  private handleInteractivePrompt(userMessages: UserAgentMessage[]): void {
    const pendingTool = getActivePromptToolName()
    const firstMessage = userMessages[0]

    // Extract text from the first message
    const messageText = this.extractTextFromMessage(firstMessage)

    logger.info(
      { taskId: this.config.taskId, pendingTool, messageLength: messageText.length },
      'MessagePollingConsumer: Agent waiting for input, using incoming message as response',
    )

    const response = convertMessageToUserInputResponse(pendingTool, messageText)
    const resolved = resolveInteractivePrompt(response)

    if (resolved) {
      logger.info(
        { taskId: this.config.taskId, pendingTool },
        'MessagePollingConsumer: Pending input resolved with message',
      )
    } else {
      logger.warn(
        { taskId: this.config.taskId, pendingTool },
        'MessagePollingConsumer: Failed to resolve pending input',
      )
    }
  }

  /**
   * Convert pending user messages to an AgentRunRequest
   */
  private convertToAgentRunRequest(
    userMessages: UserAgentMessage[],
    lastResult: ResultAgentMessage | undefined,
  ): AgentRunRequest {
    if (userMessages.length === 0) {
      throw new Error('Cannot create request from empty messages')
    }

    // Use the last user message for options (most recent intent)
    const lastUserMessage = userMessages[userMessages.length - 1]

    // Accumulate content from all messages
    const content: AgentRunRequest['content'] = []
    for (const msg of userMessages) {
      content.push(...this.extractContentFromMessage(msg))
    }

    // Derive resume from lastResult
    const resume = this.deriveResumeSessionId(lastResult)

    return {
      content,
      agentMode: lastUserMessage.agentMode,
      model: lastUserMessage.model,
      resume,
    }
  }

  /**
   * Convert a single user message to an AgentRunRequest (for batching)
   */
  private convertSingleMessageToRequest(
    msg: UserAgentMessage,
    lastResult: ResultAgentMessage | undefined,
  ): AgentRunRequest {
    return {
      content: this.extractContentFromMessage(msg),
      agentMode: msg.agentMode,
      model: msg.model,
      resume: this.deriveResumeSessionId(lastResult),
    }
  }

  /**
   * Extract content blocks from a UserAgentMessage
   */
  private extractContentFromMessage(msg: UserAgentMessage): AgentRunRequest['content'] {
    const apiMessage = msg.message
    if (!apiMessage?.content) {
      return [{ type: 'text', text: '' }]
    }

    if (typeof apiMessage.content === 'string') {
      return [{ type: 'text', text: apiMessage.content }]
    }

    // Content is an array - extract text blocks
    const content: AgentRunRequest['content'] = []
    for (const block of apiMessage.content) {
      if ('text' in block && block.type === 'text') {
        content.push({ type: 'text', text: block.text })
      }
    }

    return content.length > 0 ? content : [{ type: 'text', text: '' }]
  }

  /**
   * Extract text string from a UserAgentMessage
   */
  private extractTextFromMessage(msg: UserAgentMessage): string {
    const apiMessage = msg.message
    if (!apiMessage?.content) {
      return ''
    }

    if (typeof apiMessage.content === 'string') {
      return apiMessage.content
    }

    return apiMessage.content
      .filter((c): c is { type: 'text'; text: string } => 'text' in c && c.type === 'text')
      .map((c) => c.text)
      .join('\n')
  }

  /**
   * Derive the resume session ID from the last result message
   * If the last result indicates the agent can resume, use the agentSessionId
   */
  private deriveResumeSessionId(lastResult: ResultAgentMessage | undefined): string | undefined {
    if (!lastResult) {
      return undefined
    }

    // If agent explicitly said it will not resume, don't resume
    if (lastResult.willNotResume) {
      return undefined
    }

    // Use the agentSessionId from the last result for resume
    return lastResult.agentSessionId
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}

/**
 * Create and initialize the message polling consumer
 */
export function createMessagePollingConsumer(
  config: SandboxConfig,
  eventService: SandboxEventService | undefined,
): MessagePollingConsumer {
  const consumer = new MessagePollingConsumer(config, eventService)

  logger.info(
    {
      taskId: config.taskId,
      backendApiUrl: config.backendApiUrl,
    },
    'Message polling consumer initialized',
  )

  return consumer
}
