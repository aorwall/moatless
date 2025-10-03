import type { AgentEvent, BranchNameAndSummary, Event } from '@moatless/types'
import { type AgentOptions, type AgentRunOptions, BaseAgent } from './base-agent.js'

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
   * Main run method that generates test events
   */
  async *run(message: string, runOptions: AgentRunOptions = {}): AsyncGenerator<Event> {
    this.stopRequested = false
    this.executionCount++
    this.capturedRunOptions = { ...runOptions }
    this.iterationNumber = runOptions.iterationNumber

    // Determine which model to use (same priority as ClaudeCodeAgent)
    const model = runOptions.model || this.options.model || 'default-test-model'

    // Set session ID
    this.sessionId = `test-session-${Date.now()}-${this.executionCount}`

    // Emit start event
    yield this.createEvent('agent', 'started', {
      message: 'Test agent starting',
      model,
      sessionId: this.sessionId,
    })

    // If configured to error, emit error event
    if (this.shouldError) {
      yield this.createEvent(
        'agent',
        'error',
        {
          error: this.errorMessage,
          model,
        },
        true,
      )
      return
    }

    // Emit message event simulating agent response
    yield this.createEvent('agent', 'message', {
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
    })

    // Check if stop was requested
    if (this.stopRequested) {
      yield this.createEvent('agent', 'stopped', {
        message: 'Test agent stopped by user',
        model,
      })
      return
    }

    // Emit any custom events if configured
    for (const event of this.responseEvents) {
      yield event
      if (this.stopRequested) break
    }

    // Emit completion event
    yield this.createEvent('agent', 'completed', {
      message: 'Test completed successfully',
      model,
      inputMessage: message,
      totalTokens: 150,
    })
  }

  /**
   * Stop the agent
   */
  async stop(): Promise<void> {
    this.stopRequested = true
  }

  /**
   * Get the agent type
   */
  getAgentType(): string {
    return 'test-agent'
  }

  /**
   * Generate a branch name and summary for testing
   */
  async getBranchNameAndSummary(
    message: string,
    options?: AgentRunOptions,
  ): Promise<BranchNameAndSummary | null> {
    const model = options?.model || this.options.model || 'default-test-model'
    return {
      branchName: `test-branch-${Date.now()}`,
      summary: `Test branch for: "${message}" using model: ${model}`,
    }
  }

  // Test configuration methods

  /**
   * Configure the agent to error on next run
   */
  configureError(shouldError: boolean, errorMessage?: string): void {
    this.shouldError = shouldError
    if (errorMessage) {
      this.errorMessage = errorMessage
    }
  }

  /**
   * Add custom events to be emitted during run
   */
  addResponseEvents(events: Event[]): void {
    this.responseEvents = events
  }

  /**
   * Clear all response events
   */
  clearResponseEvents(): void {
    this.responseEvents = []
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
