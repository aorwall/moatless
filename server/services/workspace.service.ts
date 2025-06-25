import { randomUUID } from 'node:crypto'
import { EnvironmentNotFoundError, WorkspaceError } from '@shared/constants/errors.js'
import type { FastifyInstance } from 'fastify'
import type { CommandResult, ServerStatus, Workspace } from '../../shared/types/index.js'
import { logger } from '../utils/logger.js'
import type { BaseWorkspace } from '../workspaces/base.js'
import { DockerWorkspace } from '../workspaces/docker.js'
import { LocalWorkspace } from '../workspaces/local.js'
import type { BaseAgent } from './agents/base-agent.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, BaseWorkspace> = new Map()

  constructor(
    private environmentService: EnvironmentService,
    private eventService: EventService,
  ) {}

  async getOrCreateWorkspace(
    taskId: string,
    environmentId: string,
    branchName?: string,
  ): Promise<BaseWorkspace> {
    const workspaceLogger = logger.child({ taskId, environmentId })

    workspaceLogger.info('Creating workspace', {
      taskId,
      environmentId,
      hasBranchName: !!branchName,
    })

    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId)
    if (existing) {
      workspaceLogger.debug('Workspace already exists, returning existing instance')
      return existing
    }

    // Get environment
    const environment = await this.environmentService.getEnvironment(environmentId)
    if (!environment) {
      workspaceLogger.error('Environment not found', undefined, { environmentId })
      throw new EnvironmentNotFoundError(environmentId)
    }

    workspaceLogger.debug('Environment loaded successfully', {
      environmentType: environment.repository.type,
      codeExecutionType: environment.codeExecution.type,
      useGitWorktrees:
        environment.repository.type === 'local' ? environment.repository.useGitWorktrees : false,
    })

    // Create workspace instance based on code execution type
    let workspace: BaseWorkspace
    const startTime = Date.now()

    try {
      if (environment.codeExecution.type === 'docker') {
        workspaceLogger.debug('Creating Docker workspace')
        workspace = new DockerWorkspace(taskId, environment, this.eventService)
      } else {
        workspaceLogger.debug('Creating Local workspace', { branchName })
        workspace = new LocalWorkspace(
          taskId,
          environment,
          this.eventService,
          undefined,
          branchName,
        )
      }

      const creationTime = Date.now() - startTime
      workspaceLogger.debug('Workspace instance created successfully', {
        workspaceType: workspace.type,
        creationTimeMs: creationTime,
      })

      this.activeWorkspaces.set(taskId, workspace)
      workspaceLogger.debug('Workspace added to active workspaces map')

      // Workspace created and stored in activeWorkspaces

      workspaceLogger.info('Workspace creation completed successfully', {
        workspaceType: workspace.type,
        totalTimeMs: Date.now() - startTime,
      })

      return workspace
    } catch (error) {
      const failureTime = Date.now() - startTime
      workspaceLogger.error('Workspace creation failed', error as Error, {
        timeToFailureMs: failureTime,
        environmentType: environment.repository.type,
        codeExecutionType: environment.codeExecution.type,
      })
      throw error
    }
  }

  async getWorkspace(taskId: string): Promise<BaseWorkspace | null> {
    // Check active workspaces only
    return this.activeWorkspaces.get(taskId) || null
  }

  async listWorkspaces(): Promise<Workspace[]> {
    const workspaces: Workspace[] = []
    for (const [_taskId, workspace] of this.activeWorkspaces) {
      workspaces.push(workspace.toWorkspaceInfo())
    }
    return workspaces
  }

  async updateWorkspaceStatus(
    taskId: string,
    _status: Workspace['status'],
    _error?: string,
  ): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    // Status is managed by the workspace instance itself
    // No storage needed as we verify existence through activeWorkspaces
  }

  async listFiles(
    taskId: string,
    relativePath: string = '.',
    recursive: boolean = false,
  ): Promise<string[]> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    return workspace.listFiles(relativePath, recursive)
  }

  async readFile(taskId: string, filePath: string): Promise<string> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    return workspace.readFile(filePath)
  }

  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    await workspace.writeFile(filePath, content)

    // Emit file change event
    await this.eventService.emit(
      randomUUID(),
      'workspace',
      taskId,
      new Date(),
      false,
      { filePath, action: 'write' },
      'file_changed',
    )
  }

  async getGitDiff(taskId: string): Promise<any> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    return workspace.getGitDiff()
  }

  async cleanupWorkspace(taskId: string): Promise<void> {
    const workspaceLogger = logger.child({ taskId })

    workspaceLogger.info('Starting workspace cleanup')

    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      workspaceLogger.debug('No workspace found for cleanup')
      return
    }

    const startTime = Date.now()
    try {
      workspaceLogger.debug('Initiating workspace teardown', {
        workspaceType: workspace.type,
        force: true,
      })

      await workspace.teardown(true) // Force cleanup

      const teardownTime = Date.now() - startTime
      workspaceLogger.debug('Workspace teardown completed', { teardownTimeMs: teardownTime })

      // Remove from active workspaces
      this.activeWorkspaces.delete(taskId)
      workspaceLogger.debug('Workspace removed from active workspaces map')

      workspaceLogger.info('Workspace cleanup completed successfully', {
        totalTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      workspaceLogger.error('Workspace cleanup failed', error as Error, {
        timeToFailureMs: failureTime,
        workspaceType: workspace.type,
      })
      throw error
    }
  }

  async setupWorkspace(taskId: string, iterationNumber?: number): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    await workspace.setup(iterationNumber)
  }

  async triggerAgent(
    taskId: string,
    agentType: string,
    message: string,
    iterationNumber: number,
  ): Promise<BaseAgent> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    return workspace.triggerAgent(agentType, message, iterationNumber)
  }

  async runCommand(taskId: string, command: string, timeout?: number): Promise<CommandResult> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    return workspace.runCommand(command, timeout)
  }

  async getWorkspaceStatus(taskId: string): Promise<any> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    return {
      status: workspace.getStatus(),
      hasUncommittedChanges: await workspace.hasUncommittedChanges(),
    }
  }

  async startServer(taskId: string): Promise<any> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (!workspace.startServer) {
      throw new WorkspaceError(`Server not supported for this workspace type`, taskId)
    }

    return workspace.startServer()
  }

  async stopServer(taskId: string): Promise<boolean> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (!workspace.stopServer) {
      return false
    }

    return workspace.stopServer()
  }

  async getServerStatus(taskId: string): Promise<ServerStatus> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (!workspace.getServerStatus) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }

    return workspace.getServerStatus()
  }

  async getServerLogs(taskId: string, lines?: number): Promise<string> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    if (!workspace.getServerLogs) {
      return 'Server logs not available for this workspace type'
    }

    return workspace.getServerLogs(lines)
  }
}

export function createWorkspaceService(
  _app: FastifyInstance,
  environmentService: EnvironmentService,
  eventService: EventService,
): WorkspaceService {
  return new WorkspaceService(environmentService, eventService)
}
