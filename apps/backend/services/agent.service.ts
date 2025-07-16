import { randomUUID } from 'node:crypto'
import type { SDKResultMessage } from '@moatless/types'
import { EventType, type TaskIteration, TaskNotFoundError, WorkspaceStatus } from '@moatless/types'
import type { BaseAgent } from '../agents/base-agent.js'
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

    if (status === WorkspaceStatus.SETTING_UP) {
      throw new Error('Workspace is still setting up')
    }
    if (status === WorkspaceStatus.RUNNING) {
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

      if (recreatedStatus !== WorkspaceStatus.IDLE) {
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
      // Update task status to running
      const task = await this.storage.getTask(taskId)
      if (task) {
        task.status = 'running'
        await this.storage.saveTask(task)
      }

      // Execute agent in background
      await this.runAgentExecution(agent, message, taskId, iteration, workspace, resumeSessionId)
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
          type: EventType.TASK_STATUS,
          subType: 'open',
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
   * Process agent execution and handle events
   */
  private async runAgentExecution(
    agent: BaseAgent,
    message: string,
    taskId: string,
    iteration: TaskIteration,
    workspace: Awaited<ReturnType<WorkspaceService['getOrCreateWorkspace']>>,
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

      // Process agent events using agent's run method directly
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
              // Mark as will not resume when max turns is reached
              iteration.willNotResume = true
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

          // Capture both git status and git changes when iteration completes
          try {
            // Get git status for sync state
            iteration.gitStatus = await workspace.getGitStatus()
            agentLogger.debug('Captured git status for iteration', {
              syncStatus: iteration.gitStatus?.status,
              commitsAhead: iteration.gitStatus?.commitsAhead,
              commitsBehind: iteration.gitStatus?.commitsBehind,
              hasUncommittedChanges: iteration.gitStatus?.hasUncommittedChanges,
            })

            // Get git changes for working directory state
            iteration.gitChanges = await workspace.getGitChanges()
            agentLogger.debug('Captured git changes for iteration', {
              currentBranch: iteration.gitChanges?.currentBranch,
              status: iteration.gitChanges?.status,
              additions: iteration.gitChanges?.additions,
              deletions: iteration.gitChanges?.deletions,
              modifications: iteration.gitChanges?.modifications,
            })

            // Get current commit hash
            iteration.commitHash = (await workspace.getCurrentCommitHash()) || undefined
            agentLogger.debug('Captured commit hash for iteration', {
              commitHash: iteration.commitHash,
            })
          } catch (error) {
            agentLogger.warn('Failed to capture git status/changes for iteration', {
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
              // Update the iteration data
              task.iterations[iterationIndex] = iteration
              // Update task status based on iteration result
              const previousStatus = task.status
              const newStatus = event.isError || resultPayload.is_error ? 'error' : 'open'
              task.status = newStatus
              await this.storage.saveTask(task)

              // Emit task status change event if status actually changed
              if (previousStatus !== newStatus) {
                await this.eventService.emitEvent({
                  taskId,
                  type: EventType.TASK_STATUS,
                  subType: newStatus,
                  payload: {
                    status: newStatus,
                    previousStatus,
                    iterationNumber: iteration.iterationNumber,
                  },
                  eventId: randomUUID(),
                  timestamp: new Date(),
                  agentType: task.agentType,
                  isError: newStatus === 'error',
                })
              }
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

      // Capture both git status and git changes even for failed iterations
      try {
        // Get git status for sync state
        iteration.gitStatus = await workspace.getGitStatus()
        agentLogger.debug('Captured git status for failed iteration', {
          syncStatus: iteration.gitStatus?.status,
          commitsAhead: iteration.gitStatus?.commitsAhead,
          commitsBehind: iteration.gitStatus?.commitsBehind,
          hasUncommittedChanges: iteration.gitStatus?.hasUncommittedChanges,
        })

        // Get git changes for working directory state
        iteration.gitChanges = await workspace.getGitChanges()
        agentLogger.debug('Captured git changes for failed iteration', {
          currentBranch: iteration.gitChanges?.currentBranch,
          status: iteration.gitChanges?.status,
          additions: iteration.gitChanges?.additions,
          deletions: iteration.gitChanges?.deletions,
          modifications: iteration.gitChanges?.modifications,
        })

        // Get current commit hash
        iteration.commitHash = (await workspace.getCurrentCommitHash()) || undefined
        agentLogger.debug('Captured commit hash for failed iteration', {
          commitHash: iteration.commitHash,
        })
      } catch (syncError) {
        agentLogger.warn('Failed to capture git status/changes for failed iteration', {
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
          // Update the iteration data
          task.iterations[iterationIndex] = iteration
          // Update task status to error after failure
          const previousStatus = task.status
          task.status = 'error'
          await this.storage.saveTask(task)

          // Emit task status change event if status actually changed
          if (previousStatus !== 'error') {
            await this.eventService.emitEvent({
              taskId,
              type: EventType.TASK_STATUS,
              subType: 'error',
              payload: {
                status: 'error',
                previousStatus,
                iterationNumber: iteration.iterationNumber,
                error: error instanceof Error ? error.message : String(error),
              },
              eventId: randomUUID(),
              timestamp: new Date(),
              agentType: task.agentType,
              isError: true,
            })
          }
        }
      }

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
      if (workspace && workspace.getStatus() === WorkspaceStatus.IDLE) {
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
