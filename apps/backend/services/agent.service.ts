import { randomUUID } from 'node:crypto'
import { CriticalError } from '@moatless/shared/constants/errors'
import type { SDKResultMessage } from '@moatless/shared/types/claude'
import { EventType, type TaskIteration } from '@moatless/shared/types/index'
import { WorkspaceStatus } from '@moatless/shared/types/workspace'
import type { BaseAgent } from '../services/agents/base-agent.js'
import { ClaudeCodeAgent } from '../services/agents/claude-code-agent.js'
import { logger } from '../utils/logger.js'
import type { BaseWorkspace } from '../workspaces/base.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export class AgentService {
  private runningAgents: Map<string, BaseAgent> = new Map()

  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private workspaceService: WorkspaceService,
  ) {}

  /**
   * Execute an agent for a task
   */
  async executeAgent(
    taskId: string,
    message: string,
    iteration: TaskIteration,
    agentType: string = 'claude-code',
    resumeSessionId?: string,
    model?: string,
  ): Promise<void> {
    const agentLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    // Validate that no agent is currently running for this task
    if (this.runningAgents.has(taskId)) {
      throw new CriticalError('Agent is already running', taskId, iteration)
    }

    // Get workspace and validate state
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const status = workspace.getStatus()

    if (status === WorkspaceStatus.SETTING_UP) {
      throw new CriticalError('Workspace is still setting up', taskId, iteration)
    }
    if (status === WorkspaceStatus.RUNNING) {
      throw new CriticalError('Agent is already running', taskId, iteration)
    }

    // Check if workspace path actually exists
    if (!(await workspace.isWorkspacePathValid())) {
      // Workspace metadata exists but physical path/container is missing
      // Need to recreate the workspace
      await this.recreateWorkspace(taskId, iteration)

      // Get the workspace again after recreation
      const recreatedWorkspace = await this.workspaceService.getOrCreateWorkspace(taskId)
      const recreatedStatus = recreatedWorkspace.getStatus()

      if (recreatedStatus !== WorkspaceStatus.IDLE) {
        throw new CriticalError(
          'Workspace recreation failed - workspace not ready',
          taskId,
          iteration,
        )
      }
    }

    agentLogger.info('Creating agent', {
      agentType,
      workspaceStatus: status,
      hasResumeSession: !!resumeSessionId,
      model,
    })

    // Create agent
    const agent = await this.createAgent(agentType, taskId, model)

    // Track the running agent
    this.runningAgents.set(taskId, agent)

    try {
      // Update task status to running
      const task = await this.storage.getTask(taskId)
      if (task) {
        task.status = 'running'
        await this.storage.saveTask(task)
      }

      // Execute agent in background
      await this.processAgentExecution(
        agent,
        message,
        taskId,
        iteration,
        workspace,
        resumeSessionId,
      )
    } finally {
      // Always clean up running agent tracking
      this.runningAgents.delete(taskId)

      // Update task status back to open if not error
      const task = await this.storage.getTask(taskId)
      if (task && task.status === 'running') {
        const previousStatus = task.status
        task.status = 'open'
        await this.storage.saveTask(task)

        // Emit status change event
        await this.eventService.emitEvent({
          taskId,
          type: EventType.INPUT,
          subType: 'task:statusChanged',
          payload: {
            status: 'open',
            previousStatus,
          },
          eventId: randomUUID(),
          timestamp: new Date(),
          agentType: task.agentType,
          isError: false,
        })
      }
    }
  }

  /**
   * Stop a running agent
   */
  async stopAgent(taskId: string): Promise<boolean> {
    const agent = this.runningAgents.get(taskId)
    if (!agent) {
      return false
    }

    logger.info('Stopping agent execution', { taskId })

    try {
      // Stop the agent if it has a stop method
      if (typeof agent.stop === 'function') {
        await agent.stop()
      }

      // Remove from tracking
      this.runningAgents.delete(taskId)

      logger.info('Agent stopped successfully', { taskId })
      return true
    } catch (error) {
      logger.error('Failed to stop agent', error as Error, { taskId })
      return false
    }
  }

  /**
   * Get the current running status of a task.
   * This is used by other services to check if agent is still running.
   */
  async getTaskRunningStatus(taskId: string): Promise<boolean> {
    return this.runningAgents.has(taskId)
  }

  /**
   * Create an agent instance
   */
  private async createAgent(agentType: string, taskId: string, model?: string): Promise<BaseAgent> {
    if (agentType === 'claude-code') {
      // TODO: Get claude options from storage/config
      const options = model ? { model } : {} // Pass model in options if provided
      return new ClaudeCodeAgent(taskId, options)
    } else {
      throw new Error(`Unknown agent type: ${agentType}`)
    }
  }

  /**
   * Process agent execution and handle events
   */
  private async processAgentExecution(
    agent: BaseAgent,
    message: string,
    taskId: string,
    iteration: TaskIteration,
    workspace: BaseWorkspace,
    resumeSessionId?: string,
  ): Promise<void> {
    const agentLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    try {
      const runOptions = {
        workspaceDir: workspace.getWorkspacePath(),
        iterationNumber: iteration.iterationNumber,
        resume: resumeSessionId,
      }

      agentLogger.info('Starting agent execution with options', runOptions)

      // Prepare message with file references if provided
      let finalMessage = message
      if (iteration.attachments && iteration.attachments.length > 0) {
        // Get the absolute paths for the uploaded files
        const filePaths: string[] = iteration.attachments.map((attachment) => attachment.path)

        // Log each attachment
        for (const attachment of iteration.attachments) {
          agentLogger.info('File attachment included', {
            filename: attachment.filename,
            path: attachment.path,
            mimeType: attachment.mimeType,
          })
        }

        // Append file information to message
        finalMessage = `${message}\n\nUploaded files:\n${filePaths.join('\n')}`
        agentLogger.info('Message with file attachments prepared', {
          fileCount: filePaths.length,
          messageLength: finalMessage.length,
        })
      }

      // Process agent events
      for await (const event of agent.run(finalMessage, runOptions)) {
        // Capture session ID if it's not already set
        if (!iteration.agentSessionId && agent.getSessionId()) {
          iteration.agentSessionId = agent.getSessionId()

          agentLogger.debug('Captured agent session ID', { sessionId: iteration.agentSessionId })

          // Update the session ID in the task's iteration
          const task = await this.storage.getTask(taskId)
          if (task) {
            const iterationIndex = task.iterations.findIndex(
              (it) => it.iterationNumber === iteration.iterationNumber,
            )
            if (iterationIndex >= 0) {
              task.iterations[iterationIndex].agentSessionId = iteration.agentSessionId
              await this.storage.saveTask(task)
            }
          }
        }

        // Update contextTokens if provided in the event
        if (event.contextTokens !== undefined) {
          iteration.contextTokens = event.contextTokens
          agentLogger.debug('Updated context tokens', { contextTokens: event.contextTokens })
        }

        if (event.type === EventType.RESULT) {
          const resultPayload = event.payload as SDKResultMessage

          agentLogger.info('Agent execution completed', {
            costUsd: resultPayload.total_cost_usd,
            hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
            isError: event.isError,
            subtype: resultPayload.subtype,
          })

          iteration.costUsd = resultPayload.total_cost_usd ?? 0

          // Check if the result event indicates an error
          if (event.isError || resultPayload.is_error) {
            // Handle error result
            iteration.status = 'failed'

            // Determine error message based on subtype
            let errorMessage = 'Agent execution failed'
            if (resultPayload.subtype === 'error_max_turns') {
              errorMessage = 'Maximum number of turns exceeded'
            } else if (resultPayload.subtype === 'error_during_execution') {
              errorMessage = 'Error occurred during execution'
            }

            iteration.error = {
              message: errorMessage,
              timestamp: new Date(),
            }

            if ('result' in resultPayload) {
              iteration.result = resultPayload.result
            }
          } else {
            // Handle successful result
            iteration.status = 'completed'
            // Type guard to ensure we're dealing with a success result
            if ('result' in resultPayload) {
              iteration.result =
                resultPayload.result || 'No result found in Claude Code SDK result message'
            } else {
              iteration.result = 'No result found in Claude Code SDK result message'
            }
          }

          iteration.endTime = new Date()
          iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

          // Capture git sync status when iteration completes
          try {
            if (
              'getOriginSyncStatus' in workspace &&
              typeof workspace.getOriginSyncStatus === 'function'
            ) {
              iteration.gitSyncStatus = await workspace.getOriginSyncStatus()
              agentLogger.debug('Captured git sync status for iteration', {
                syncStatus: iteration.gitSyncStatus?.status,
                commitsAhead: iteration.gitSyncStatus?.commitsAhead,
                commitsBehind: iteration.gitSyncStatus?.commitsBehind,
                hasUncommittedChanges: iteration.gitSyncStatus?.hasUncommittedChanges,
              })
            }
          } catch (error) {
            agentLogger.warn('Failed to capture git sync status for iteration', {
              error: error instanceof Error ? error.message : String(error),
            })
          }

          // Find and update the iteration in the task's iterations array
          const task = await this.storage.getTask(taskId)
          if (task) {
            const iterationIndex = task.iterations.findIndex(
              (it) => it.iterationNumber === iteration.iterationNumber,
            )
            if (iterationIndex >= 0) {
              // Remove runtime properties to avoid circular references
              const { task: _, taskId: __, ...iterationData } = iteration
              task.iterations[iterationIndex] = iterationData
              // Update task status based on iteration result
              task.status = event.isError || resultPayload.is_error ? 'error' : 'open'
              await this.storage.saveTask(task)
            }
          }
        }

        // Emit agent events to event service
        agentLogger.debug('Emitting agent event to event service', { eventType: event.type })
        await this.eventService.emitEvent(event)
      }

      agentLogger.info('Agent execution completed successfully', {
        iterationNumber: iteration.iterationNumber,
        status: iteration.status,
      })
    } catch (error) {
      agentLogger.error('Agent execution failed', error as Error)

      // Save error to iteration
      iteration.status = 'failed'
      iteration.error = {
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
        timestamp: new Date(),
      }
      iteration.endTime = new Date()
      iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

      // Capture git sync status even for failed iterations
      try {
        if (
          'getOriginSyncStatus' in workspace &&
          typeof workspace.getOriginSyncStatus === 'function'
        ) {
          iteration.gitSyncStatus = await workspace.getOriginSyncStatus()
          agentLogger.debug('Captured git sync status for failed iteration', {
            syncStatus: iteration.gitSyncStatus?.status,
            commitsAhead: iteration.gitSyncStatus?.commitsAhead,
            commitsBehind: iteration.gitSyncStatus?.commitsBehind,
            hasUncommittedChanges: iteration.gitSyncStatus?.hasUncommittedChanges,
          })
        }
      } catch (syncError) {
        agentLogger.warn('Failed to capture git sync status for failed iteration', {
          syncError: syncError instanceof Error ? syncError.message : String(syncError),
        })
      }

      // Find and update the iteration in the task's iterations array
      const task = await this.storage.getTask(taskId)
      if (task) {
        const iterationIndex = task.iterations.findIndex(
          (it) => it.iterationNumber === iteration.iterationNumber,
        )
        if (iterationIndex >= 0) {
          // Remove runtime properties to avoid circular references
          const { task: _, taskId: __, ...iterationData } = iteration
          task.iterations[iterationIndex] = iterationData
          // Update task status to error after failure
          task.status = 'error'
          await this.storage.saveTask(task)
        }
      }

      // Re-throw critical errors
      if (error instanceof CriticalError) {
        throw error
      }
    }
  }

  /**
   * Recreate workspace when physical path/container is missing
   */
  private async recreateWorkspace(taskId: string, iteration: TaskIteration): Promise<void> {
    const agentLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    agentLogger.warn('Workspace path/container missing, recreating workspace', { taskId })

    // Setup workspace again
    await this.workspaceService.setupWorkspace(taskId, iteration.iterationNumber)

    // Wait for workspace to be ready
    const maxWaitTime = 30000
    const checkInterval = 500
    const startWait = Date.now()

    while (Date.now() - startWait < maxWaitTime) {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace && workspace.getStatus() === WorkspaceStatus.IDLE) {
        agentLogger.info('Workspace recreated successfully', { taskId })
        return
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    throw new CriticalError('Workspace recreation timeout', taskId, iteration)
  }
}

export function createAgentService(
  storage: StorageBackend,
  eventService: EventService,
  workspaceService: WorkspaceService,
): AgentService {
  return new AgentService(storage, eventService, workspaceService)
}
