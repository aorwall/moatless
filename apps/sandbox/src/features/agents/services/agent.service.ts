import crypto from 'node:crypto'
import type { AgentRunRequest } from '@moatless/sandbox-schemas'
import type { Event, UserContent } from '@moatless/types'
import { logger } from '../../../shared/logger'
import type { SandboxEventService } from '../../events/services/sandbox-event.service'
import type { SandboxTool } from '../../tools/types'
import type { AgentEventCallback } from '../base/base-agent'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent'

/**
 * Batched agent request combining multiple queued requests
 *
 * Content is accumulated from ALL requests in order.
 * Options are taken from the LAST request only (most recent user intent).
 *
 * Note: Static configuration (plugins, timeout, systemPrompt) is read from
 * REPO_CONFIG.agentConfig by the agent, not passed per-request.
 */
export interface BatchedAgentRequest {
  /** Combined content from ALL requests */
  content: UserContent[]
  /** Options from LAST request (per-request only) */
  agentMode: AgentRunRequest['agentMode']
  model: AgentRunRequest['model']
  resume: AgentRunRequest['resume']
  /** Original requests for reference */
  originalRequests: AgentRunRequest[]
  /** Number of requests in this batch */
  batchSize: number
}

/**
 * Result of agent execution
 */
export interface AgentExecutionResult {
  success: boolean
  sessionId?: string
  error?: string
}

/**
 * AgentService - Centralized agent execution and event management
 *
 * This service:
 * 1. Extracts common logic from agent.routes.ts and agent-request-consumer.ts
 * 2. Provides batched request execution for multiple queued requests
 * 3. Manages session ID tracking for resume functionality
 * 4. Handles event callback creation and emission
 */
export class AgentService {
  private currentAgent: ClaudeCodeAgent | undefined
  private agentState: 'idle' | 'running' = 'idle'
  private lastSessionId: string | undefined
  private lastError: string | undefined

  constructor(
    private readonly taskId: string,
    private readonly workspaceDir: string,
    private readonly eventService: SandboxEventService | undefined,
    private readonly tools?: SandboxTool[],
  ) {}

  /**
   * Execute a single agent request
   */
  async executeRequest(request: AgentRunRequest): Promise<AgentExecutionResult> {
    const resolvedRequest = this.resolveResumeOption(request)

    // Create agent with resolved request
    this.currentAgent = new ClaudeCodeAgent(this.taskId, {
      model: resolvedRequest.model,
      sandboxTools: this.tools,
    })
    this.agentState = 'running'
    this.lastError = undefined

    const messageText = this.extractMessageText(resolvedRequest.content)

    logger.info(
      {
        taskId: this.taskId,
        messageLength: messageText.length,
        model: resolvedRequest.model,
        hasEventService: !!this.eventService,
        resume: resolvedRequest.resume,
        agentMode: resolvedRequest.agentMode,
      },
      'AgentService: Starting agent execution',
    )

    try {
      const onEvent = this.createEventCallback()
      await this.currentAgent.run(resolvedRequest, this.workspaceDir, onEvent)

      // Capture session ID for future resume requests
      const capturedSessionId = this.currentAgent.getSessionId()
      if (capturedSessionId) {
        this.lastSessionId = capturedSessionId
        logger.debug(
          { taskId: this.taskId, sessionId: capturedSessionId },
          'AgentService: Session ID captured for resume',
        )
      }

      logger.info(
        { taskId: this.taskId, sessionId: capturedSessionId },
        'AgentService: Agent execution completed successfully',
      )

      return { success: true, sessionId: capturedSessionId }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.lastError = errorMessage

      logger.error({ err: error, taskId: this.taskId }, 'AgentService: Agent execution error')

      await this.emitErrorEvent(errorMessage)

      return { success: false, error: errorMessage }
    } finally {
      this.agentState = 'idle'
    }
  }

  /**
   * Execute a batched agent request
   *
   * This processes multiple queued requests as a single SDK call,
   * yielding an SDKUserMessage for each request in the batch.
   */
  async executeBatchedRequest(batchedRequest: BatchedAgentRequest): Promise<AgentExecutionResult> {
    // Resolve resume option using last session ID if needed
    const resolvedResume = this.resolveResumeValue(batchedRequest.resume)

    // Create agent with batched request options (from last request)
    this.currentAgent = new ClaudeCodeAgent(this.taskId, {
      model: batchedRequest.model,
      sandboxTools: this.tools,
    })
    this.agentState = 'running'
    this.lastError = undefined

    const totalContent = batchedRequest.content.length
    const messageText = this.extractMessageText(batchedRequest.content)

    logger.info(
      {
        taskId: this.taskId,
        batchSize: batchedRequest.batchSize,
        totalContentBlocks: totalContent,
        messageLength: messageText.length,
        model: batchedRequest.model,
        hasEventService: !!this.eventService,
        resume: resolvedResume,
        agentMode: batchedRequest.agentMode,
      },
      'AgentService: Starting batched agent execution',
    )

    try {
      const onEvent = this.createEventCallback()

      // Create a single AgentRunRequest from the batched request
      // Note: plugins, timeout, systemPrompt are read from REPO_CONFIG by the agent
      const combinedRequest: AgentRunRequest = {
        content: batchedRequest.content,
        agentMode: batchedRequest.agentMode,
        model: batchedRequest.model,
        resume: resolvedResume,
      }

      // Use runBatched if batch size > 1, otherwise use regular run
      if (batchedRequest.batchSize > 1) {
        await this.currentAgent.runBatched(
          batchedRequest.originalRequests,
          this.workspaceDir,
          onEvent,
        )
      } else {
        await this.currentAgent.run(combinedRequest, this.workspaceDir, onEvent)
      }

      // Capture session ID for future resume requests
      const capturedSessionId = this.currentAgent.getSessionId()
      if (capturedSessionId) {
        this.lastSessionId = capturedSessionId
        logger.debug(
          { taskId: this.taskId, sessionId: capturedSessionId },
          'AgentService: Session ID captured for resume',
        )
      }

      logger.info(
        { taskId: this.taskId, sessionId: capturedSessionId, batchSize: batchedRequest.batchSize },
        'AgentService: Batched agent execution completed successfully',
      )

      return { success: true, sessionId: capturedSessionId }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.lastError = errorMessage

      logger.error(
        { err: error, taskId: this.taskId, batchSize: batchedRequest.batchSize },
        'AgentService: Batched agent execution error',
      )

      await this.emitErrorEvent(errorMessage)

      return { success: false, error: errorMessage }
    } finally {
      this.agentState = 'idle'
    }
  }

  /**
   * Create a batched request from multiple queued requests
   *
   * Content is accumulated from ALL requests in order.
   * Options are taken from the LAST request (most recent user intent).
   *
   * Note: Static config (plugins, timeout, systemPrompt) is read from
   * REPO_CONFIG by the agent, not passed per-request.
   */
  createBatchedRequest(requests: AgentRunRequest[]): BatchedAgentRequest {
    if (requests.length === 0) {
      throw new Error('Cannot create batched request from empty array')
    }

    // Accumulate content from all requests
    const allContent: UserContent[] = []
    for (const request of requests) {
      allContent.push(...request.content)
    }

    // Take per-request options from the last request
    const lastRequest = requests[requests.length - 1]

    return {
      content: allContent,
      agentMode: lastRequest.agentMode,
      model: lastRequest.model,
      resume: lastRequest.resume,
      originalRequests: requests,
      batchSize: requests.length,
    }
  }

  /**
   * Stop the currently running agent
   */
  async stopAgent(): Promise<void> {
    if (this.currentAgent && this.agentState === 'running') {
      await this.currentAgent.stop()
      this.agentState = 'idle'
    }
  }

  /**
   * Get current agent state
   */
  getState(): { state: 'idle' | 'running'; hasAgent: boolean; error?: string } {
    return {
      state: this.agentState,
      hasAgent: this.currentAgent !== undefined,
      error: this.lastError,
    }
  }

  /**
   * Get the last captured session ID for resume functionality
   */
  getLastSessionId(): string | undefined {
    return this.lastSessionId
  }

  /**
   * Extract text message from content array
   * Combines all text content blocks into a single message
   */
  extractMessageText(content: UserContent[]): string {
    if (!content || content.length === 0) {
      return ''
    }
    return content
      .filter((c): c is { type: 'text'; text: string } => c.type === 'text')
      .map((c) => c.text)
      .join('\n')
  }

  /**
   * Create event callback for agent execution
   */
  private createEventCallback(): AgentEventCallback {
    return async (event: Event) => {
      if (this.eventService) {
        try {
          await this.eventService.emitEvent(event)
        } catch (emitError) {
          logger.warn(
            { err: emitError, taskId: this.taskId, eventType: event.type },
            'AgentService: Failed to emit event to Redis',
          )
        }
      }
      logger.debug(
        { taskId: this.taskId, eventType: event.type },
        'AgentService: Agent event processed',
      )
    }
  }

  /**
   * Emit an error event
   */
  private async emitErrorEvent(errorMessage: string): Promise<void> {
    if (this.eventService) {
      try {
        await this.eventService.emitEvent({
          eventId: crypto.randomUUID(),
          type: 'agent',
          subType: 'error',
          taskId: this.taskId,
          agentType: 'claude-code',
          timestamp: new Date(),
          isError: true,
          payload: { error: errorMessage },
        } as Event)
      } catch (emitError) {
        logger.warn(
          { err: emitError, taskId: this.taskId },
          'AgentService: Failed to emit error event to Redis',
        )
      }
    }
  }

  /**
   * Resolve resume option to actual session ID
   *
   * When resume is boolean true, use the last captured session ID.
   * When resume is a string, use that session ID directly.
   * When resume is false/undefined, return undefined (new session).
   */
  private resolveResumeOption(request: AgentRunRequest): AgentRunRequest {
    const resolvedResume = this.resolveResumeValue(request.resume)

    if (resolvedResume !== request.resume) {
      logger.info(
        {
          taskId: this.taskId,
          originalResume: request.resume,
          resolvedResume,
          lastSessionId: this.lastSessionId,
        },
        'AgentService: Resume option resolved',
      )
      return { ...request, resume: resolvedResume }
    }

    return request
  }

  /**
   * Resolve a resume value to a session ID string or undefined
   */
  private resolveResumeValue(resume: boolean | string | undefined): string | undefined {
    if (resume === true && this.lastSessionId) {
      return this.lastSessionId
    }

    if (resume === true && !this.lastSessionId) {
      logger.warn(
        { taskId: this.taskId },
        'AgentService: resume=true but no session ID available, starting new session',
      )
      return undefined
    }

    if (typeof resume === 'string') {
      return resume
    }

    return undefined
  }
}
