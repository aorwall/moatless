import { EnvironmentNotFoundError, WorkspaceError } from '@shared/constants/errors.js'
import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import type { CommandResult, ServerStatus, Workspace } from '../../shared/types/index.js'
import { LocalWorkspace } from '../workspaces/local.js'
import { DockerWorkspace } from '../workspaces/docker.js'
import type { BaseWorkspace } from '../workspaces/base.js'
import type { EnvironmentService } from './environment.service.js'
import type { EventService } from './event.service.js'
import type { StorageBackend } from './storage.js'
import type { BaseAgent } from './agents/base-agent.js'

export class WorkspaceService {
  private activeWorkspaces: Map<string, BaseWorkspace> = new Map()

  constructor(
    private storage: StorageBackend,
    private environmentService: EnvironmentService,
    private eventService: EventService,
  ) {}

  async createWorkspace(
    taskId: string,
    environmentId: string,
    branchName?: string,
  ): Promise<BaseWorkspace> {
    // Check if workspace already exists
    const existing = this.activeWorkspaces.get(taskId)
    if (existing) return existing

    // Get environment
    const environment = await this.environmentService.getEnvironment(environmentId)
    if (!environment) {
      throw new EnvironmentNotFoundError(environmentId)
    }

    // Create workspace instance based on code execution type
    let workspace: BaseWorkspace
    if (environment.codeExecution.type === 'docker') {
      workspace = new DockerWorkspace(taskId, environment, this.eventService)
    } else {
      workspace = new LocalWorkspace(taskId, environment, this.eventService, undefined, branchName)
    }

    this.activeWorkspaces.set(taskId, workspace)
    
    // Store workspace info
    await this.storage.saveWorkspace(workspace.toWorkspaceInfo())

    return workspace
  }

  async getWorkspace(taskId: string): Promise<BaseWorkspace | null> {
    // Check active workspaces first
    const active = this.activeWorkspaces.get(taskId)
    if (active) return active

    // Load from storage and recreate workspace instance
    const workspaceInfo = await this.storage.getWorkspace(taskId)
    if (workspaceInfo) {
      const environment = await this.environmentService.getEnvironment(workspaceInfo.environmentId)
      if (!environment) {
        throw new EnvironmentNotFoundError(workspaceInfo.environmentId)
      }

      let workspace: BaseWorkspace
      if (workspaceInfo.type === 'docker') {
        workspace = new DockerWorkspace(taskId, environment, this.eventService)
      } else {
        workspace = new LocalWorkspace(taskId, environment, this.eventService)
      }

      this.activeWorkspaces.set(taskId, workspace)
      return workspace
    }
    return null
  }

  async listWorkspaces(): Promise<Workspace[]> {
    const workspaceKeys = await this.storage.list('workspaces/')
    const workspaces: Workspace[] = []

    for (const key of workspaceKeys) {
      const workspaceId = key.replace('workspaces/', '')
      const workspace = await this.storage.getWorkspace(workspaceId)
      if (workspace) {
        workspaces.push(workspace)
      }
    }

    return workspaces
  }

  async updateWorkspaceStatus(
    taskId: string,
    status: Workspace['status'],
    error?: string,
  ): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new WorkspaceError(`Workspace not found for task ${taskId}`, taskId)
    }

    // Get current workspace info and update it
    const workspaceInfo = workspace.toWorkspaceInfo()
    workspaceInfo.status = status
    if (error) {
      workspaceInfo.error = error
    }

    await this.storage.saveWorkspace(workspaceInfo)
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
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) return

    try {
      await workspace.teardown(true) // Force cleanup

      // Remove from active workspaces and storage
      this.activeWorkspaces.delete(taskId)
      await this.storage.delete(`workspaces/${taskId}`)
    } catch (error) {
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
  storage: StorageBackend,
  environmentService: EnvironmentService,
  eventService: EventService,
): WorkspaceService {
  return new WorkspaceService(storage, environmentService, eventService)
}
