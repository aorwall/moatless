import type { GitChanges, GitStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { FileOperations } from '../../../services/file-operations.service'
import { getCurrentCommitHash } from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import { WorkspaceCoordinator } from './workspace-coordinator.service'

/**
 * FileGateway - Bridge between TaskService and file operations
 *
 * This service provides file operation methods that TaskService needs,
 * coordinating with WorkspaceCoordinator to ensure workspace is provisioned
 * before performing operations.
 *
 * Responsibilities:
 * - Ensure workspace exists before file operations
 * - Delegate file operations to FileOperations service
 * - Provide git status and changes information
 */
@injectable()
export class FileGateway {
  constructor(
    @inject(WorkspaceCoordinator)
    private readonly coordinator: WorkspaceCoordinator,
    // FileOperations will be created per workspace path
  ) {}

  /**
   * Get workspace path for a task
   */
  async getWorkspacePath(taskId: string): Promise<string> {
    const handle = await this.coordinator.ensureWorkspace(taskId)
    return handle.workspacePath
  }

  /**
   * Get git status for a workspace
   */
  async getGitStatus(taskId: string): Promise<GitStatus> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Getting git status', { taskId, workspacePath })

    return await fileOps.getGitStatus()
  }

  /**
   * Get git changes for a workspace
   */
  async getGitChanges(taskId: string): Promise<GitChanges> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Getting git changes', { taskId, workspacePath })

    return await fileOps.getGitChanges()
  }

  /**
   * Get current commit hash for a workspace
   */
  async getCurrentCommitHash(taskId: string): Promise<string | null> {
    const workspacePath = await this.getWorkspacePath(taskId)

    logger.debug('FileGateway: Getting current commit hash', {
      taskId,
      workspacePath,
    })

    try {
      return await getCurrentCommitHash(workspacePath)
    } catch (error) {
      logger.warn('FileGateway: Failed to get commit hash', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  }
}
