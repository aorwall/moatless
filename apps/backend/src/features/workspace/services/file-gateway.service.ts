import type { FileTreeWithChangesResponse, GitChanges, GitStatus } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseEnvironmentDriver, FileSearchRequest } from '../drivers/base-environment-driver'
import { WorkspaceRuntimeManager } from './workspace-runtime-manager.service'

/**
 * FileGateway - Bridge between TaskService and file operations
 *
 * This service provides a unified file operation interface that accepts only taskId
 * as the workspace identifier. It delegates to WorkspaceRuntimeManager for driver
 * management and ensureWorkspace for workspace provisioning.
 *
 * Responsibilities:
 * - Provide taskId-only API for file operations
 * - Coordinate with WorkspaceRuntimeManager for driver access
 * - Route file operations to appropriate driver (local or Kubernetes)
 * - Provide git status, changes, and diff information
 *
 * Design Principle: TaskId-Only API
 * Consumers call methods with just taskId; FileGateway handles all path resolution
 * and execution mode routing internally via WorkspaceRuntimeManager.
 *
 * Note: Driver creation and caching is now handled by WorkspaceRuntimeManager.
 * FileGateway acts as a thin wrapper for backward compatibility.
 */
@injectable()
export class FileGateway {
  constructor(
    @inject(WorkspaceRuntimeManager)
    private readonly runtimeManager: WorkspaceRuntimeManager,
  ) {}

  /**
   * Get the environment driver for a task
   * Delegates to WorkspaceRuntimeManager.ensureWorkspace() to get driver from handle
   */
  async getDriver(taskId: string): Promise<BaseEnvironmentDriver> {
    const handle = await this.runtimeManager.ensureWorkspace(taskId)
    return handle.driver
  }

  /**
   * Stop agent and cleanup cached driver for a task
   * Delegates to WorkspaceRuntimeManager
   */
  async stopAgentAndInvalidate(taskId: string): Promise<void> {
    await this.runtimeManager.stopAgentAndInvalidate(taskId)
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
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId }, 'FileGateway: Getting git status')

    return await driver.getGitStatus()
  }

  /**
   * Get git changes for a workspace
   */
  async getGitChanges(taskId: string): Promise<GitChanges> {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId }, 'FileGateway: Getting git changes')

    return await driver.getGitChanges()
  }

  /**
   * Get current commit hash for a workspace
   */
  async getCurrentCommitHash(taskId: string): Promise<string | null> {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId }, 'FileGateway: Getting current commit hash')

    return await driver.getCurrentCommitHash()
  }

  /**
   * Check if workspace is valid and accessible
   * In Kubernetes mode, this checks if the workspace handle exists and is healthy
   */
  async isWorkspacePathValid(taskId: string): Promise<boolean> {
    try {
      // Check if workspace handle exists and is healthy
      const handle = await this.runtimeManager.getExistingWorkspaceHandle(taskId)
      if (!handle) {
        logger.debug({ taskId }, 'FileGateway: No workspace handle found')
        return false
      }

      logger.debug({ taskId }, 'FileGateway: Workspace handle exists and is valid')
      return true
    } catch (error) {
      logger.warn(
        {
          taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'FileGateway: Workspace validation failed',
      )
      return false
    }
  }

  /**
   * Read a file from the workspace
   */
  async readFile(taskId: string, filePath: string): Promise<string> {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, filePath }, 'FileGateway: Reading file')

    return await driver.readFile(filePath)
  }

  /**
   * Write a file to the workspace
   */
  async writeFile(taskId: string, filePath: string, content: string): Promise<void> {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, filePath }, 'FileGateway: Writing file')

    await driver.writeFile(filePath, content)
  }

  /**
   * List files in the workspace
   */
  async listFiles(taskId: string, relativePath = '.', recursive = false): Promise<string[]> {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, relativePath, recursive }, 'FileGateway: Listing files')

    return await driver.listFiles(relativePath, recursive)
  }

  /**
   * Search for files in the workspace using ripgrep
   */
  async searchFiles(taskId: string, request: FileSearchRequest) {
    const driver = await this.getDriver(taskId)

    logger.debug(
      {
        taskId,
        query: request.query,
        limit: request.limit,
      },
      'FileGateway: Searching files',
    )

    return await driver.searchFiles(request)
  }

  /**
   * Get git diff for the workspace
   */
  async getGitDiff(taskId: string) {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId }, 'FileGateway: Getting git diff')

    return await driver.getGitDiff()
  }

  /**
   * Read a file with git change information
   */
  async readFileWithChanges(taskId: string, filePath: string) {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, filePath }, 'FileGateway: Reading file with changes')

    return await driver.readFileWithChanges(filePath)
  }

  /**
   * Get file tree with git change information
   * This combines file listing and git diff data in a single performant call
   */
  async getFileTreeWithChanges(
    taskId: string,
    rootPath = '.',
  ): Promise<FileTreeWithChangesResponse> {
    const driver = await this.getDriver(taskId)

    logger.debug({ taskId, rootPath }, 'FileGateway: Getting file tree with changes')

    return await driver.getFileTreeWithChanges(rootPath)
  }
}
