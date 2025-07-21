import { randomUUID } from 'node:crypto'
import type { SDKResultMessage } from '@moatless/types'
import { type TaskIteration, TaskNotFoundError } from '@moatless/types'

import type { AgentRunOptions, BaseAgent } from '../agents/base-agent.js'
import { logger } from '../utils/logger.js'
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
    agentType = 'claude-code',
    resumeSessionId?: string,
    model?: string,
  ): Promise<void> {
    // Get task and repository config
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError('Task not found')
    }

    // Execute agent directly
    await this.processAgentExecution(taskId, message, iteration, agentType, resumeSessionId, model)
  }

  /**
   * Process agent execution locally (original implementation)
   */
  private async processAgentExecution(
    taskId: string,
    message: string,
    iteration: TaskIteration,
    agentType = 'claude-code',
    resumeSessionId?: string,
    model?: string,
  ): Promise<void> {
    const agentLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    // Validate that no agent is currently running for this task
    if (this.runningAgents.has(taskId)) {
      throw new Error('Agent is already running')
    }

    // Get workspace and validate state
    agentLogger.info('Getting or creating workspace for agent execution', { taskId })
    const workspace = await this.workspaceService.getOrCreateWorkspace(taskId)
    const status = workspace.getStatus()

    agentLogger.debug('Workspace status retrieved', {
      taskId,
      workspaceStatus: status,
      workspaceType: workspace.type,
    })

    if (status === 'initializing') {
      throw new Error('Workspace is still setting up')
    }
    if (status === 'ready' && workspace.runningAgent) {
      throw new Error('Agent is already running')
    }

    // Check if workspace path actually exists
    const isWorkspaceValid = await workspace.isWorkspacePathValid()
    agentLogger.debug('Workspace path validation', {
      taskId,
      isValid: isWorkspaceValid,
      workspacePath: workspace.getWorkspacePath(),
    })

    if (!isWorkspaceValid) {
      // Workspace metadata exists but physical path/container is missing
      // Need to recreate the workspace
      agentLogger.warn('Workspace path invalid, recreating workspace', { taskId })
      await this.recreateWorkspace(taskId, iteration)

      // Get the workspace again after recreation
      const recreatedWorkspace = await this.workspaceService.getOrCreateWorkspace(taskId)
      const recreatedStatus = recreatedWorkspace.getStatus()

      agentLogger.info('Workspace recreated', {
        taskId,
        newStatus: recreatedStatus,
      })

      if (recreatedStatus !== 'ready') {
        throw new Error('Workspace recreation failed - workspace not ready')
      }
    }

    agentLogger.info('Getting environment from workspace', {
      agentType,
      workspaceStatus: status,
      hasResumeSession: !!resumeSessionId,
      model,
    })

    // Get environment from workspace
    const environment = workspace.getEnvironment()

    // Create agent using environment
    const options = model ? { model } : {}
    const agent = await environment.createAgent(agentType, options)

    // Track the running agent
    this.runningAgents.set(taskId, agent)

    try {
      // Execute agent in background (task status remains 'open' during execution)
      await this.runAgentExecution(
        agent,
        message,
        taskId,
        iteration,
        workspace,
        resumeSessionId,
        model,
      )
    } finally {
      // Always clean up running agent tracking
      this.runningAgents.delete(taskId)

      // Agent execution complete - task stays in its current state (open/closed/error)
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
   * Process agent execution and handle events
   */
  private async runAgentExecution(
    agent: BaseAgent,
    message: string,
    taskId: string,
    iteration: TaskIteration,
    workspace: Awaited<ReturnType<WorkspaceService['getOrCreateWorkspace']>>,
    resumeSessionId?: string,
    model?: string,
  ): Promise<void> {
    const agentLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    try {
      const runOptions: AgentRunOptions = {
        workspaceDir: workspace.getWorkspacePath(),
        iterationNumber: iteration.iterationNumber,
        resume: resumeSessionId,
        model,
      }

      agentLogger.info('Starting agent execution with options', { ...runOptions })

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

      // Process agent events using agent's run method directly
      for await (const event of agent.run(finalMessage, runOptions)) {
        // Capture session ID if it's not already set
        if (!iteration.agentSessionId && agent.getSessionId()) {
          iteration.agentSessionId = agent.getSessionId()
          agentLogger.debug('Captured agent session ID', { sessionId: iteration.agentSessionId })
        }

        // Context tokens will be updated by task service via event handling

        if (event.type === 'result') {
          const resultPayload = event.payload as SDKResultMessage

          agentLogger.info('Agent execution completed', {
            costUsd: resultPayload.total_cost_usd,
            hasResult: resultPayload.subtype === 'success' && 'result' in resultPayload,
            isError: event.isError,
            subtype: resultPayload.subtype,
          })

          // Set basic iteration data for local tracking
          iteration.costUsd = resultPayload.total_cost_usd ?? 0
          iteration.endTime = new Date()
          iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()
        }

        // Emit agent events to event service with agentSessionId
        agentLogger.debug('Emitting agent event to event service', { eventType: event.type })
        await this.eventService.emitEvent({
          ...event,
          agentSessionId: iteration.agentSessionId,
        })
      }

      agentLogger.info('Agent execution completed successfully', {
        iterationNumber: iteration.iterationNumber,
        status: iteration.status,
      })
    } catch (error) {
      agentLogger.error('Agent execution failed', error as Error)

      // Set basic iteration data for local tracking
      iteration.status = 'failed'
      iteration.endTime = new Date()
      iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

      // Emit error event - task service will handle the task/iteration updates
      await this.eventService.emitEvent({
        taskId,
        type: 'agent',
        subType: 'error',
        payload: {
          error: error instanceof Error ? error.message : String(error),
          stackTrace: error instanceof Error ? error.stack : undefined,
        },
        eventId: randomUUID(),
        timestamp: new Date(),
        agentType: 'claude-code',
        isError: true,
        iterationId: iteration.iterationNumber,
        agentSessionId: iteration.agentSessionId,
      })

      throw error
    }
  }

  /**
   * Recreate workspace when physical path/container is missing
   */
  private async recreateWorkspace(taskId: string, iteration: TaskIteration): Promise<void> {
    const agentLogger = logger.child({ taskId, iterationNumber: iteration.iterationNumber })

    agentLogger.warn('Workspace path/container missing, recreating workspace', { taskId })

    // Setup workspace again
    agentLogger.debug('Calling setupWorkspace to recreate', {
      taskId,
      iterationNumber: iteration.iterationNumber,
    })

    await this.workspaceService.setupWorkspace(taskId, iteration.iterationNumber)

    // Wait for workspace to be ready
    const maxWaitTime = 30000
    const checkInterval = 500
    const startWait = Date.now()

    agentLogger.debug('Waiting for workspace to become ready', {
      maxWaitTime,
      checkInterval,
    })

    while (Date.now() - startWait < maxWaitTime) {
      const workspace = await this.workspaceService.getWorkspace(taskId)
      if (workspace && workspace.getStatus() === 'ready') {
        const waitTime = Date.now() - startWait
        agentLogger.info('Workspace recreated successfully', {
          taskId,
          waitTimeMs: waitTime,
          workspaceStatus: workspace.getStatus(),
        })
        return
      }
      await new Promise((resolve) => setTimeout(resolve, checkInterval))
    }

    throw new Error('Workspace recreation timeout')
  }
}

export function createAgentService(
  storage: StorageBackend,
  eventService: EventService,
  workspaceService: WorkspaceService,
): AgentService {
  return new AgentService(storage, eventService, workspaceService)
}
