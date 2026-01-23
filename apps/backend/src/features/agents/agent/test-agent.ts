import type { AgentEvent, Event } from '@moatless/types'
import { type AgentOptions, type AgentRunOptions, BaseAgent } from './base-agent'

/**
 * TestAgent - A stub mock agent for testing purposes
 *
 * This agent replicates the behavior of ClaudeCodeAgent for testing,
 * particularly for verifying that model parameters are passed correctly
 * through the system.
 */
export class TestAgent extends BaseAgent {
  // Public properties for test verification
  public capturedOptions: AgentOptions = {}
  public capturedRunOptions: AgentRunOptions = {}
  public executionCount = 0

  // Allow tests to configure response behavior
  private responseEvents: Event[] = []
  private shouldError = false
  private errorMessage = 'Test error'

  constructor(taskId: string, options: AgentOptions = {}) {
    super(taskId, options)
    this.capturedOptions = { ...options }
  }

  /**
   * Main run method that emits test events via callback
   */
  // biome-ignore lint/suspicious/useAwait: async required to satisfy abstract Promise<void> signature
  async run(message: string, runOptions: AgentRunOptions = {}): Promise<void> {
    this.stopRequested = false
    this.executionCount++
    this.capturedRunOptions = { ...runOptions }
    this.iterationNumber = runOptions.iterationNumber
    this.onEvent = runOptions.onEvent

    // Determine which model to use (same priority as ClaudeCodeAgent)
    const model = runOptions.model || this.options.model || 'default-test-model'

    // Set session ID
    this.sessionId = `test-session-${Date.now()}-${this.executionCount}`

    // Emit start event
    this.emitEvent(
      this.createEvent('agent', 'started', {
        message: 'Test agent starting',
        model,
        sessionId: this.sessionId,
      }),
    )

    // If configured to error, emit error event
    if (this.shouldError) {
      this.emitEvent(
        this.createEvent(
          'agent',
          'error',
          {
            error: this.errorMessage,
            model,
          },
          true,
        ),
      )
      return
    }

    // Emit message event simulating agent response
    this.emitEvent(
      this.createEvent('agent', 'message', {
        type: 'assistant',
        message: {
          id: `msg-test-${this.executionCount}`,
          type: 'message',
          role: 'assistant',
          model,
          content: [
            {
              type: 'text',
              text: `Test response for: "${message}" using model: ${model}`,
            },
          ],
          stop_reason: 'end_turn',
          stop_sequence: null,
          usage: {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
          },
        },
        parent_tool_use_id: null,
        session_id: this.sessionId,
      }),
    )

    // Check if stop was requested
    if (this.stopRequested) {
      this.emitEvent(
        this.createEvent('agent', 'stopped', {
          message: 'Test agent stopped by user',
          model,
        }),
      )
      return
    }

    // Emit any custom events if configured
    for (const event of this.responseEvents) {
      this.emitEvent(event)
      if (this.stopRequested) {
        break
      }
    }

    // Emit completion event
    this.emitEvent(
      this.createEvent('agent', 'completed', {
        message: 'Test completed successfully',
        model,
        inputMessage: message,
        totalTokens: 150,
      }),
    )
  }

  /**
   * Stop the agent
   */
  stop(): Promise<void> {
    this.stopRequested = true
    return Promise.resolve()
  }

  /**
   * Get the agent type
   */
  getAgentType(): string {
    return 'test-agent'
  }

  /**
   * Get the effective model being used
   */
  getEffectiveModel(runOptions?: AgentRunOptions): string {
    return runOptions?.model || this.options.model || 'default-test-model'
  }

  // Helper method to create events with consistent structure
  private createEvent(
    type: 'agent',
    subType: string,
    payload: Record<string, unknown>,
    isError = false,
  ): AgentEvent {
    return {
      eventId: `test-event-${type}-${subType}-${Date.now()}-${Math.random().toString(36).substring(7)}`,
      agentType: 'test-agent',
      type,
      subType,
      taskId: this.taskId,
      iterationId: this.iterationNumber,
      timestamp: new Date(),
      isError,
      payload: {
        ...payload,
        agentSessionId: this.sessionId,
      },
    } as AgentEvent
  }
}
