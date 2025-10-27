import type { FileTreeWithChangesResponse, GitChanges, GitStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { FileOperations, type FileSearchRequest } from '../../../services/file-operations.service'
import { getCurrentCommitHash } from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import { WorkspaceRuntimeManager } from './workspace-runtime-manager.service'

/**
 * FileGateway - Bridge between TaskService and file operations
 *
 * This service provides a unified file operation interface that accepts only taskId
 * as the workspace identifier. This design hides workspace path resolution and
 * execution target routing from consumers (TaskService, file routes).
 *
 * Responsibilities:
 * - Coordinate with WorkspaceRuntimeManager to resolve taskId → workspacePath
 * - Delegate file operations to FileOperations service (local mode)
 * - Provide git status, changes, and diff information
 *
 * Design Principle: TaskId-Only API
 * Consumers call methods with just taskId; FileGateway handles all path resolution
 * and execution mode routing internally.
 */
@injectable()
export class FileGateway {
  constructor(
    @inject(WorkspaceRuntimeManager)
    private readonly runtimeManager: WorkspaceRuntimeManager,
    // FileOperations will be created per workspace path
  ) {}

  /**
   * Get workspace path for a task
   * Coordinates with WorkspaceRuntimeManager to resolve taskId → workspacePath
   */
  async getWorkspacePath(taskId: string): Promise<string> {
    return await this.runtimeManager.getWorkspacePath(taskId)
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

  /**
   * Check if workspace path is valid and accessible
   */
  async isWorkspacePathValid(taskId: string): Promise<boolean> {
    try {
      const workspacePath = await this.getWorkspacePath(taskId)
      const fileOps = new FileOperations(taskId, workspacePath)

      logger.debug('FileGateway: Checking workspace path validity', {
        taskId,
        workspacePath,
      })

      return await fileOps.isWorkspacePathValid()
    } catch (error) {
      logger.warn('FileGateway: Workspace path validation failed', {
        taskId,
        error: error instanceof Error ? error.message : String(error),
      })
      return false
    }
  }

  /**
   * Read a file from the workspace
   */
  async readFile(taskId: string, filePath: string): Promise<string> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Reading file', { taskId, filePath })

    return await fileOps.readFile(filePath)
  }

  /**
   * Write a file to the workspace
   */
  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Writing file', { taskId, filePath })

    await fileOps.writeFile(filePath, content)
  }

  /**
   * List files in the workspace
   */
  async listFiles(taskId: string, relativePath = '.', recursive = false): Promise<string[]> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Listing files', { taskId, relativePath, recursive })

    return await fileOps.listFiles(relativePath, recursive)
  }

  /**
   * Search for files in the workspace using ripgrep
   */
  async searchFiles(taskId: string, request: FileSearchRequest) {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Searching files', {
      taskId,
      query: request.query,
      limit: request.limit,
    })

    return await fileOps.searchFiles(request)
  }

  /**
   * Get git diff for the workspace
   */
  async getGitDiff(taskId: string) {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Getting git diff', { taskId, workspacePath })

    return await fileOps.getGitDiff()
  }

  /**
   * Read a file with git change information
   */
  async readFileWithChanges(taskId: string, filePath: string) {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug('FileGateway: Reading file with changes', { taskId, filePath })

    return await fileOps.readFileWithChanges(filePath)
  }

  /**
   * Get file tree with git change information
   * This combines file listing and git diff data in a single performant call
   */
  async getFileTreeWithChanges(
    taskId: string,
    rootPath = '.',
  ): Promise<FileTreeWithChangesResponse> {
    const workspacePath = await this.getWorkspacePath(taskId)

    logger.debug('FileGateway: Getting file tree with changes', { taskId, rootPath })

    // Import workspace utilities dynamically to avoid circular dependencies
    const { getGitDiffDetailed } = await import('../../../utils/git-utils.js')

    // Get FileOperations instance for file listing
    const fileOps = new FileOperations(taskId, workspacePath)

    // Parallel execution for performance
    const [gitDiff, allFiles] = await Promise.all([
      getGitDiffDetailed(workspacePath, undefined), // No original branch parameter needed
      fileOps.listFiles(rootPath, true), // recursive = true
    ])

    // Create a map of file paths to git diff information for O(1) lookup
    const gitFileMap = new Map<
      string,
      {
        status: 'added' | 'modified' | 'deleted' | 'renamed'
        additions: number
        deletions: number
        isStaged: boolean
      }
    >()

    let totalStaged = 0
    let totalModified = 0

    for (const file of gitDiff.files) {
      gitFileMap.set(file.filename, {
        status: file.status,
        additions: file.additions,
        deletions: file.deletions,
        isStaged: file.isStaged,
      })

      if (file.isStaged) {
        totalStaged++
      } else {
        totalModified++
      }
    }

    // Build tree structure with git information
    const tree = allFiles.map((filePath) => {
      const gitInfo = gitFileMap.get(filePath)
      return {
        name: filePath.split('/').pop() || filePath,
        path: filePath,
        type: 'file' as const,
        isChanged: !!gitInfo,
        status: gitInfo?.status,
        additions: gitInfo?.additions,
        deletions: gitInfo?.deletions,
        isStaged: gitInfo?.isStaged,
      }
    })

    return {
      tree,
      gitSummary: {
        totalFiles: gitDiff.files.length,
        totalAdditions: gitDiff.totalAdditions,
        totalDeletions: gitDiff.totalDeletions,
        totalStaged,
        totalModified,
        compareBase: gitDiff.compareBase || 'HEAD',
        hasChanges: gitDiff.files.length > 0,
      },
    }
  }
}
