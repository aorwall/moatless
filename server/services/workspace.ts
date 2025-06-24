import type { FastifyInstance } from 'fastify'
import type { LocalWorkspace, Workspace } from '../../shared/types/index.js'
import type { StorageBackend } from './storage.js'

export class WorkspaceService {
  constructor(private storage: StorageBackend) {}

  async createWorkspace(
    taskId: string,
    environmentId: string,
    _branchName?: string,
  ): Promise<Workspace> {
    const workspace: LocalWorkspace = {
      id: taskId, // Use taskId as workspace ID for simplicity
      type: 'local',
      environmentId,
      status: 'initializing',
      createdAt: new Date(),
      path: `/tmp/workspaces/${taskId}`, // Temporary path
    }

    await this.storage.saveWorkspace(workspace)
    return workspace
  }

  async getWorkspace(taskId: string): Promise<Workspace | null> {
    return this.storage.getWorkspace(taskId)
  }

  async updateWorkspace(taskId: string, updates: Partial<Workspace>): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      throw new Error(`Workspace for task ${taskId} not found`)
    }

    // Handle discriminated union properly
    const updatedWorkspace =
      workspace.type === 'local'
        ? ({ ...workspace, ...updates } as LocalWorkspace)
        : ({ ...workspace, ...updates } as Workspace)

    await this.storage.saveWorkspace(updatedWorkspace)
  }

  async cleanupWorkspace(taskId: string): Promise<void> {
    const workspace = await this.getWorkspace(taskId)
    if (!workspace) {
      return
    }

    // Update status to cleaning
    await this.updateWorkspace(taskId, { status: 'cleaning' })

    // TODO: Implement actual cleanup logic (delete files, stop containers, etc.)

    // Delete workspace record
    await this.storage.delete(`workspaces/${taskId}`)
  }
}

export function createWorkspaceService(
  _app: FastifyInstance,
  storage: StorageBackend,
): WorkspaceService {
  return new WorkspaceService(storage)
}
