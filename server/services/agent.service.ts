import { ValidationError } from '@shared/constants/errors.js'
import { randomUUID } from 'crypto'
import type { FastifyInstance } from 'fastify'
import { validateAgentOptions } from './agents/agent-options.js'
import type { BaseAgent } from './agents/base-agent.js'
import { ClaudeCodeAgent } from './agents/claude-code-agent.js'
import type { EventService } from './event.service.js'
import type { TaskService } from './task.service.js'
import type { WorkspaceService } from './workspace.service.js'

export type AgentType = 'claude-code' | 'openai' | 'custom'

export interface AgentExecution {
  id: string
  taskId: string
  agentType: AgentType
  status: 'running' | 'completed' | 'error' | 'stopped'
  startedAt: Date
  completedAt?: Date
  error?: string
  agent?: BaseAgent
}

export class AgentService {
  private activeAgents: Map<string, AgentExecution> = new Map()
  private agentRegistry: Map<AgentType, typeof BaseAgent> = new Map()

  constructor(
    private taskService: TaskService,
    private workspaceService: WorkspaceService,
    private eventService: EventService,
  ) {
    this.registerDefaultAgents()
  }

  async runAgent(
    taskId: string,
    message: string,
    agentType: AgentType = 'claude-code',
    options: any = {},
  ): Promise<string> {
    // Check if agent is already running for this task
    const existingExecution = Array.from(this.activeAgents.values()).find(
      (exec) => exec.taskId === taskId && exec.status === 'running',
    )

    if (existingExecution) {
      throw new ValidationError('Agent is already running for this task', {
        taskId,
        executionId: existingExecution.id,
      })
    }

    // Get task metadata
    const task = await this.taskService.getTask(taskId)
    if (!task) {
      throw new ValidationError(`Task ${taskId} not found`, { taskId })
    }

    // Get workspace
    const workspace = await this.workspaceService.getWorkspace(taskId)
    if (!workspace) {
      throw new ValidationError(`Workspace for task ${taskId} not found`, { taskId })
    }

    // Validate options
    const validationErrors = validateAgentOptions(options)
    if (validationErrors.length > 0) {
      throw new ValidationError('Invalid agent options', { errors: validationErrors })
    }

    // Create agent execution record
    const executionId = randomUUID()
    const execution: AgentExecution = {
      id: executionId,
      taskId,
      agentType,
      status: 'running',
      startedAt: new Date(),
    }

    this.activeAgents.set(executionId, execution)

    // Create and run agent asynchronously
    this.runAgentAsync(execution, workspace, message, options).catch((error) => {
      console.error(`Agent execution ${executionId} failed:`, error)
    })

    return executionId
  }

  async stopAgent(executionId: string): Promise<void> {
    const execution = this.activeAgents.get(executionId)
    if (!execution) {
      throw new ValidationError(`Agent execution ${executionId} not found`, { executionId })
    }

    if (execution.status !== 'running') {
      throw new ValidationError(`Agent execution ${executionId} is not running`, {
        executionId,
        status: execution.status,
      })
    }

    if (execution.agent) {
      await execution.agent.stop()
    }

    execution.status = 'stopped'
    execution.completedAt = new Date()

    // Emit stop event
    await this.eventService.emitEvent({
      eventId: randomUUID(),
      type: 'stop' as any,
      taskId: execution.taskId,
      timestamp: new Date(),
      isError: false,
      payload: {
        executionId,
        reason: 'user_requested',
      },
    })
  }

  async getAgentExecution(executionId: string): Promise<AgentExecution | null> {
    return this.activeAgents.get(executionId) || null
  }

  async listActiveAgents(): Promise<AgentExecution[]> {
    return Array.from(this.activeAgents.values()).filter((exec) => exec.status === 'running')
  }

  async listAgentExecutions(taskId?: string): Promise<AgentExecution[]> {
    const executions = Array.from(this.activeAgents.values())

    if (taskId) {
      return executions.filter((exec) => exec.taskId === taskId)
    }

    return executions
  }

  registerAgent(type: AgentType, agentClass: typeof BaseAgent): void {
    this.agentRegistry.set(type, agentClass)
  }

  private registerDefaultAgents(): void {
    this.agentRegistry.set('claude-code', ClaudeCodeAgent as any)
    // Additional agents can be registered here
  }

  private async runAgentAsync(
    execution: AgentExecution,
    workspace: any,
    message: string,
    options: any,
  ): Promise<void> {
    try {
      // Create agent based on type
      const agent = this.createAgent(execution.agentType, execution.taskId, options)
      execution.agent = agent

      // Prepare run options
      const runOptions = {
        workspaceDir: workspace.type === 'local' ? workspace.path : undefined,
        timeout: options.timeout,
        onProgress: async (event: any) => {
          await this.eventService.emitEvent(event)
        },
      }

      // Run agent and stream events
      for await (const event of agent.run(message, runOptions)) {
        if (execution.status !== 'running') {
          break
        }

        await this.eventService.emitEvent(event)
      }

      execution.status = 'completed'
      execution.completedAt = new Date()
    } catch (error) {
      execution.status = 'error'
      execution.completedAt = new Date()
      execution.error = error instanceof Error ? error.message : String(error)

      // Emit error event
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: 'error' as any,
        taskId: execution.taskId,
        timestamp: new Date(),
        isError: true,
        payload: {
          executionId: execution.id,
          error: execution.error,
        },
      })
    } finally {
      // Clean up agent reference
      if (execution.agent) {
        execution.agent = undefined
      }
    }
  }

  private createAgent(agentType: AgentType, _taskId: string, _options: any): BaseAgent {
    const AgentClass = this.agentRegistry.get(agentType)
    if (!AgentClass) {
      throw new ValidationError(`Unknown agent type: ${agentType}`, { agentType })
    }

    switch (agentType) {
      default:
        throw new ValidationError(`Agent type ${agentType} not implemented`, { agentType })
    }
  }
}

export function createAgentService(
  _app: FastifyInstance,
  taskService: TaskService,
  workspaceService: WorkspaceService,
  eventService: EventService,
): AgentService {
  return new AgentService(taskService, workspaceService, eventService)
}
