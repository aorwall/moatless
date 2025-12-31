import type { FileTreeWithChangesResponse, GitChanges, GitStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { getCurrentCommitHash } from '../../../utils/git-utils'
import { logger } from '../../../utils/logger'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { KubernetesEnvironmentDriver } from '../drivers/kubernetes-environment-driver'
import { LocalEnvironmentDriver } from '../drivers/local-environment-driver'
import { FileOperations, type FileSearchRequest } from './file-operations.service'
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
 * - Route file operations to appropriate driver (local or Kubernetes)
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
    @inject(LocalEnvironmentDriver)
    private readonly localDriver: LocalEnvironmentDriver,
    @inject(KubernetesEnvironmentDriver)
    private readonly k8sDriver: KubernetesEnvironmentDriver,
  ) {}

  /**
   * Get the appropriate environment driver for a task
   */
  private async getDriver(taskId: string): Promise<BaseEnvironmentDriver> {
    const snapshot = await this.runtimeManager.getSnapshot(taskId)
    if (snapshot?.executionTarget.mode === 'kubernetes') {
      return this.k8sDriver
    }
    return this.localDriver
  }

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
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, workspacePath }, 'FileGateway: Getting git status')

    return await driver.getGitStatus(workspacePath)
  }

  /**
   * Get git changes for a workspace
   */
  async getGitChanges(taskId: string): Promise<GitChanges> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, workspacePath }, 'FileGateway: Getting git changes')

    return await driver.getGitChanges(workspacePath)
  }

  /**
   * Get current commit hash for a workspace
   */
  async getCurrentCommitHash(taskId: string): Promise<string | null> {
    const workspacePath = await this.getWorkspacePath(taskId)

    logger.debug(
      {
        taskId,
        workspacePath,
      },
      'FileGateway: Getting current commit hash',
    )

    try {
      return await getCurrentCommitHash(workspacePath)
    } catch (error) {
      logger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'FileGateway: Failed to get commit hash',
      )
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

      logger.debug(
        {
          taskId,
          workspacePath,
        },
        'FileGateway: Checking workspace path validity',
      )

      return await fileOps.isWorkspacePathValid()
    } catch (error) {
      logger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'FileGateway: Workspace path validation failed',
      )
      return false
    }
  }

  /**
   * Read a file from the workspace
   */
  async readFile(taskId: string, filePath: string): Promise<string> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, filePath }, 'FileGateway: Reading file')

    return await driver.readFile(workspacePath, filePath)
  }

  /**
   * Write a file to the workspace
   */
  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, filePath }, 'FileGateway: Writing file')

    await driver.writeFile(workspacePath, filePath, content)
  }

  /**
   * List files in the workspace
   */
  async listFiles(taskId: string, relativePath = '.', recursive = false): Promise<string[]> {
    const workspacePath = await this.getWorkspacePath(taskId)
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, relativePath, recursive }, 'FileGateway: Listing files')

    return await driver.listFiles(workspacePath, relativePath, recursive)
  }

  /**
   * Search for files in the workspace using ripgrep
   */
  async searchFiles(taskId: string, request: FileSearchRequest) {
    const workspacePath = await this.getWorkspacePath(taskId)
    const driver = await this.getDriver(taskId)

    logger.debug(
      {
        taskId,
        query: request.query,
        limit: request.limit,
      },
      'FileGateway: Searching files',
    )

    return await driver.searchFiles(workspacePath, request)
  }

  /**
   * Get git diff for the workspace
   */
  async getGitDiff(taskId: string) {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug({ taskId, workspacePath }, 'FileGateway: Getting git diff')

    return await fileOps.getGitDiff()
  }

  /**
   * Read a file with git change information
   */
  async readFileWithChanges(taskId: string, filePath: string) {
    const workspacePath = await this.getWorkspacePath(taskId)
    const fileOps = new FileOperations(taskId, workspacePath)

    logger.debug({ taskId, filePath }, 'FileGateway: Reading file with changes')

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

    logger.debug({ taskId, rootPath }, 'FileGateway: Getting file tree with changes')

    // Import workspace utilities dynamically to avoid circular dependencies
    const { getGitDiffDetailed } = await import('../../../utils/git-utils.js')
    const { buildFileTree } = await import('../utils/file-tree-builder.js')

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

    // Build hierarchical tree structure with git information
    const tree = buildFileTree(allFiles, gitFileMap)

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
