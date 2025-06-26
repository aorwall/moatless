import type { BaseAgent } from '@server/services/agents/base-agent.js'
import { CriticalError, TaskNotFoundError } from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import { EventType, type TaskIteration } from '../../shared/types/index.js'
import { WorkspaceStatus } from '../../shared/types/workspace.js'
import { ClaudeCodeAgent } from '../services/agents/claude-code-agent.js'
import { logger } from '../utils/logger.js'
import type { BaseWorkspace } from '../workspaces/base.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'
import type { WorkspaceService } from './workspace.service.js'

export interface AgentStatus {
  status: 'idle' | 'running' | 'preparing' | 'error'
  sessionId: string | null
  startedAt: string | null
  currentIteration: number | null
  workspaceStatus: WorkspaceStatus
  canExecute: boolean
}

export class AgentService {
  private runningAgents: Map<string, BaseAgent> = new Map()

  constructor(
    private storage: StorageBackend,
    private eventService: EventService,
    private workspaceService: WorkspaceService,
  ) {}

  /**
   * Get the current agent status for a task
   */
  async getAgentStatus(taskId: string): Promise<AgentStatus> {
    // Get task to ensure it exists
    const task = await this.storage.getTask(taskId)
    if (!task) {
      throw new TaskNotFoundError(taskId)
    }

    // Get workspace status
    const workspaceStatus = await this.workspaceService.getWorkspaceStatus(taskId)

    // Check if agent is actively running
    const runningAgent = this.runningAgents.get(taskId)

    let agentStatus: AgentStatus['status'] = 'idle'
    let sessionId: string | null = null
    let startedAt: string | null = null
    let currentIteration: number | null = null

    if (runningAgent) {
      agentStatus = 'running'
      // Get the latest iteration info from task data
      if (task.iterations && task.iterations.length > 0) {
        const latestIteration = task.iterations[task.iterations.length - 1]
        currentIteration = latestIteration.iterationNumber
        sessionId = latestIteration.agentSessionId || null
        startedAt = latestIteration.startTime.toISOString()
      }
    } else if (workspaceStatus.status === WorkspaceStatus.SETTING_UP) {
      agentStatus = 'preparing'
    } else if (workspaceStatus.status === WorkspaceStatus.NOT_SETUP) {
      agentStatus = 'error'
    } else {
      // Agent not running, but check if we have session info from last iteration
      if (task.iterations && task.iterations.length > 0) {
        const latestIteration = task.iterations[task.iterations.length - 1]
        sessionId = latestIteration.agentSessionId || null
      }
      
      // Check task status to determine agent status
      if (task.status === 'error') {
        agentStatus = 'error'
      }
    }

    return {
      status: agentStatus,
      sessionId,
      startedAt,
      currentIteration,
      workspaceStatus: workspaceStatus.status,
      canExecute: workspaceStatus.status === WorkspaceStatus.IDLE && !runningAgent,
    }
  }

  /**
   * Execute an agent for a task
   */
  async executeAgent(
    taskId: string,
    message: string,
    iteration: TaskIteration,
    agentType: string = 'claude-code',
    resumeSessionId?: string,
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

    agentLogger.info('Starting agent execution', {
      agentType,
      workspaceStatus: status,
      hasResumeSession: !!resumeSessionId,
    })

    // Create agent
    const agent = await this.createAgent(agentType, taskId)

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
        task.status = 'open'
        await this.storage.saveTask(task)
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
   * Check if an agent is running for a task
   */
  isAgentRunning(taskId: string): boolean {
    return this.runningAgents.has(taskId)
  }

  /**
   * Get the current running status of a task.
   * This is used by other services to check if agent is still running.
   */
  async getTaskRunningStatus(taskId: string): Promise<boolean> {
    return this.runningAgents.has(taskId)
  }

  /**
   * Stop all running agents (useful for shutdown)
   */
  async stopAllAgents(): Promise<void> {
    const runningTaskIds = Array.from(this.runningAgents.keys())
    logger.info('Stopping all running agents', { count: runningTaskIds.length })

    const stopPromises = runningTaskIds.map((taskId) => this.stopAgent(taskId))
    await Promise.allSettled(stopPromises)

    // Clear the map
    this.runningAgents.clear()
  }

  /**
   * Get the session ID for the latest agent execution
   */
  async getAgentSessionId(taskId: string): Promise<string | null> {
    const task = await this.storage.getTask(taskId)
    if (!task || !task.iterations || task.iterations.length === 0) {
      return null
    }

    const latestIteration = task.iterations[task.iterations.length - 1]
    return latestIteration.agentSessionId || null
  }

  /**
   * Create an agent instance
   */
  private async createAgent(agentType: string, taskId: string): Promise<BaseAgent> {
    if (agentType === 'claude-code') {
      // TODO: Get claude options from storage/config
      const options = {} // Placeholder for claude options
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

      // Process agent events
      for await (const event of agent.run(message, runOptions)) {
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

        if (event.type === EventType.RESULT) {
          agentLogger.info('Agent execution completed', {
            costUsd: event.payload.total_cost_usd,
            hasResult: !!event.payload.result,
          })

          iteration.costUsd = event.payload.total_cost_usd ?? 0
          iteration.result =
            event.payload.result ?? 'No result found in Claude Code SDK result message'

          // Update iteration status to completed after successful execution
          iteration.status = 'completed'
          iteration.endTime = new Date()
          iteration.durationMs = iteration.endTime.getTime() - iteration.startTime.getTime()

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
              // Update task status to open after successful completion
              task.status = 'open'
              await this.storage.saveTask(task)
            }
          }
        }

        // Emit agent events to event service
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
}

export function createAgentService(
  _app: FastifyInstance,
  storage: StorageBackend,
  eventService: EventService,
  workspaceService: WorkspaceService,
): AgentService {
  return new AgentService(storage, eventService, workspaceService)
}
