import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CommandResult,
  FileTreeNode,
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  RepositoryConfig,
  ServerStatus,
  Workspace as WorkspaceInfo,
  WorkspaceStatus,
  WorkspaceStatusResponse,
} from '@moatless/types'
import type { BaseEnvironment } from '../environments/base-environment.js'
import type { DockerEnvironment } from '../environments/docker-environment.js'
import type { LocalEnvironment } from '../environments/local-environment.js'
import {
  FileOperations,
  type FileSearchRequest,
  type FileSearchResponse,
} from '../services/file-operations.service.js'
import type { BaseAgent } from '../services/index.js'
import type { GitDiffResponse } from '../utils/index.js'
import { syncWithMain } from '../utils/index.js'
import { createTaskLogger } from '../utils/logger.js'

// Type guards for environment types
function isLocalEnvironment(env: BaseEnvironment): env is LocalEnvironment {
  return env.getType() === 'local'
}

function isDockerEnvironment(env: BaseEnvironment): env is DockerEnvironment {
  return env.getType() === 'docker'
}

/**
 * Unified Workspace implementation that composes FileOperations and Environment.
 * This replaces the previous LocalWorkspace and DockerWorkspace classes with a single
 * implementation that delegates file operations to FileOperations service and
 * execution operations to the appropriate Environment (Local or Docker).
 */
export class Workspace {
  private fileOperations: FileOperations
  private logger

  // Properties that match the previous BaseWorkspace interface
  get id(): string {
    return this.taskId
  }

  get type(): 'local' | 'docker' {
    return this.environment.getType()
  }

  get status(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    return this.environment.getStatus()
  }

  get createdAt(): Date {
    return this.environment.getCreatedAt()
  }

  get error(): string | undefined {
    return this.environment.getError()
  }

  get repositoryId(): string {
    return this.repositoryConfig.id
  }

  get environmentConfig(): RepositoryConfig {
    return this.repositoryConfig
  }

  get runningAgent(): BaseAgent | undefined {
    if (!this.environment.isRunning()) {
      return undefined
    }

    if (isLocalEnvironment(this.environment) || isDockerEnvironment(this.environment)) {
      return this.environment.getRunningAgent()
    }

    return undefined
  }

  constructor(
    private readonly taskId: string,
    private readonly repositoryConfig: RepositoryConfig,
    private readonly environment: BaseEnvironment,
    workspacePath?: string,
    originalBranch?: string,
  ) {
    // Initialize logger
    this.logger = createTaskLogger(taskId)

    // Initialize FileOperations with the workspace path from environment
    const initialWorkspacePath = workspacePath || environment.getWorkspacePath()
    this.fileOperations = new FileOperations(taskId, initialWorkspacePath, originalBranch)

    this.logger.debug('Workspace constructor', {
      type: this.type,
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      workspacePath: initialWorkspacePath,
      hasOriginalBranch: !!originalBranch,
    })
  }

  /**
   * Set up the workspace (environment setup, then file operations validation)
   */
  async setup(iterationNumber?: number): Promise<void> {
    this.logger.info('Starting Workspace setup', {
      type: this.type,
      repositoryId: this.repositoryConfig.id,
    })

    try {
      // Step 1: Setup the environment (this handles creation of workspace path for LocalEnvironment)
      await this.environment.setup(iterationNumber)

      // Step 2: Update file operations with the resolved workspace path
      const resolvedWorkspacePath = this.environment.getWorkspacePath()
      if (resolvedWorkspacePath) {
        this.fileOperations.setWorkspacePath(resolvedWorkspacePath)
      }

      // Step 3: Validate that workspace path is accessible for file operations
      const isValid = await this.fileOperations.isWorkspacePathValid()
      if (!isValid) {
        throw new Error(`Workspace path is not valid: ${resolvedWorkspacePath}`)
      }

      this.logger.info('Workspace setup completed', {
        type: this.type,
        workspacePath: resolvedWorkspacePath,
      })
    } catch (error) {
      this.logger.error('Workspace setup failed', error as Error, {
        type: this.type,
      })
      throw error
    }
  }

  /**
   * Clean up workspace resources
   */
  async teardown(force = false): Promise<void> {
    const startTime = Date.now()

    this.logger.info('Starting workspace teardown', {
      force,
      type: this.type,
      workspacePath: this.getWorkspacePath(),
    })

    // Check for uncommitted changes first (unless force)
    if (!force) {
      this.logger.debug('Checking for uncommitted changes')
      const hasChanges = await this.hasUncommittedChanges()
      if (hasChanges) {
        this.logger.warn('Teardown aborted due to uncommitted changes')
        throw new Error('Workspace has uncommitted changes. Use force=true to close anyway.')
      }
      this.logger.debug('No uncommitted changes detected')
    }

    // Teardown the environment (will stop agent, server, and clean up environment-specific resources)
    await this.environment.teardown(force)

    const teardownTime = Date.now() - startTime
    this.logger.info('Workspace teardown completed', { teardownTimeMs: teardownTime })
  }

  // ============================================
  // File Operations (delegated to FileOperations)
  // ============================================

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    return this.fileOperations.searchFiles(request)
  }

  async getGitDiff(): Promise<GitDiffResponse> {
    return this.fileOperations.getGitDiff()
  }

  async listFiles(relativePath?: string, recursive?: boolean): Promise<string[]> {
    return this.fileOperations.listFiles(relativePath, recursive)
  }

  async readFile(filePath: string): Promise<string> {
    return this.fileOperations.readFile(filePath)
  }

  async readFileWithChanges(filePath: string): Promise<{
    content: string
    oldContent?: string
    additions?: number
    deletions?: number
    isModified?: boolean
  }> {
    return this.fileOperations.readFileWithChanges(filePath)
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    return this.fileOperations.writeFile(filePath, content)
  }

  async hasUncommittedChanges(): Promise<boolean> {
    return this.fileOperations.hasUncommittedChanges()
  }

  async getGitStatus(): Promise<GitStatus> {
    return this.fileOperations.getGitStatus()
  }

  async getGitChanges(): Promise<GitChanges> {
    return this.fileOperations.getGitChanges()
  }

  async getCurrentCommitHash(): Promise<string | null> {
    return this.fileOperations.getCurrentCommitHash()
  }

  getWorkspacePath(): string {
    return this.fileOperations.getWorkspacePath()
  }

  async isWorkspacePathValid(): Promise<boolean> {
    return this.fileOperations.isWorkspacePathValid()
  }

  // ============================================
  // Environment Operations (delegated to Environment)
  // ============================================

  async runCommand(command: string, timeout?: number): Promise<CommandResult> {
    return this.environment.runCommand(command, timeout)
  }

  isRunning(): boolean {
    return this.environment.isRunning()
  }

  async stopAgent(): Promise<boolean> {
    return this.environment.stopAgent()
  }

  // Server methods (optional - only available if environment supports them)
  async startServer(): Promise<ServerStatus> {
    if (!this.environment.startServer) {
      throw new Error(`Server operations not supported for ${this.type} environment`)
    }
    return this.environment.startServer()
  }

  async stopServer(): Promise<boolean> {
    if (!this.environment.stopServer) {
      throw new Error(`Server operations not supported for ${this.type} environment`)
    }
    return this.environment.stopServer()
  }

  async getServerStatus(): Promise<ServerStatus> {
    if (!this.environment.getServerStatus) {
      return {
        running: false,
        configured: false,
        uptimeSeconds: 0,
      }
    }
    return this.environment.getServerStatus()
  }

  async getServerLogs(lines = 100): Promise<string> {
    if (!this.environment.getServerLogs) {
      return 'Server logs not available for this environment type'
    }
    return this.environment.getServerLogs(lines)
  }

  getPreviewPort(): number | undefined {
    if (!this.environment.getPreviewPort) {
      return undefined
    }
    return this.environment.getPreviewPort()
  }

  async proxyPreviewRequest(request: {
    method: string
    path: string
    headers: Headers
    body?: string
  }): Promise<{
    status: number
    headers: Headers
    body: string | Buffer
  }> {
    if (!this.environment.proxyPreviewRequest) {
      throw new Error(`Preview proxy not supported for ${this.type} environment`)
    }
    return this.environment.proxyPreviewRequest(request)
  }

  // ============================================
  // Status and Utility Methods
  // ============================================

  getStatus(): WorkspaceStatus {
    // Check if agent is running
    if (this.runningAgent) {
      return 'running'
    }

    // Map environment status to workspace status
    const envStatus = this.environment.getStatus()
    switch (envStatus) {
      case 'not_setup':
        return 'not_setup'
      case 'initializing':
        return 'setting_up'
      case 'ready':
        return 'idle'
      case 'error':
      case 'cleaning':
        return 'idle' // Map error/cleaning to idle for backwards compatibility
      default:
        return 'not_setup'
    }
  }

  /**
   * Convert to workspace info for API responses
   */
  toWorkspaceInfo(): WorkspaceInfo {
    const baseInfo = {
      id: this.taskId,
      repositoryId: this.repositoryConfig.id,
      status: this.status,
      createdAt: this.createdAt,
      error: this.error,
    }

    if (this.type === 'local') {
      return {
        ...baseInfo,
        type: 'local' as const,
        path: this.getWorkspacePath(),
      }
    }

    if (isDockerEnvironment(this.environment)) {
      return {
        ...baseInfo,
        type: 'docker' as const,
        containerName: this.environment.getContainerName() || '',
        port: this.environment.getPort(),
        image: this.environment.getImage(),
      }
    }

    // Fallback for unknown environment types
    return {
      ...baseInfo,
      type: 'docker' as const,
      containerName: '',
      port: undefined,
      image: undefined,
    }
  }

  /**
   * Get the environment instance
   */
  getEnvironment(): BaseEnvironment {
    return this.environment
  }

  // ============================================
  // Capability Check Methods
  // ============================================

  /**
   * Check if file operations can be performed in the workspace
   */
  async canPerformFileOperations(): Promise<boolean> {
    try {
      // Check if file operations are available (workspace path exists on disk)
      return await this.isWorkspacePathValid()
    } catch {
      return false
    }
  }

  /**
   * Check if commands can be executed in the workspace
   */
  async canExecuteCommands(): Promise<boolean> {
    try {
      const status = this.getStatus()
      // Commands can only be executed when workspace is idle
      return status === 'idle'
    } catch {
      return false
    }
  }

  /**
   * Check if server management operations can be performed
   */
  async canManageServer(): Promise<boolean> {
    try {
      const status = this.environment.getStatus()
      // Server can be managed when workspace environment is ready
      return status === 'ready'
    } catch {
      return false
    }
  }

  /**
   * Get comprehensive workspace status including git status and capabilities
   */
  async getWorkspaceStatus(backgroundTaskCount = 0): Promise<WorkspaceStatusResponse> {
    // Get git status and git changes from workspace
    const gitStatus = await this.getGitStatus()

    // Get environment information
    const environment = this.getEnvironment()
    const environmentType = environment.getType()
    const environmentStatus = environment.getStatus()
    const environmentError = environment.getError()
    const environmentCreatedAt = environment.getCreatedAt()

    // Check if file operations are available (workspace path exists on disk)
    const fileOperationsAvailable = await this.isWorkspacePathValid()

    // Determine capabilities based on environment status and file operations availability
    const capabilities = {
      fileOperations: fileOperationsAvailable,
      commandExecution: environmentStatus === 'ready',
    }

    return {
      status: this.getStatus(),
      gitStatus,
      backgroundTaskCount,
      environment: {
        type: environmentType,
        status: environmentStatus,
        error: environmentError,
        createdAt: environmentCreatedAt.toISOString(),
        capabilities,
      },
    }
  }

  /**
   * Sync workspace with main branch
   */
  async syncWithMain(baseBranch?: string): Promise<{
    success: boolean
    conflicts: boolean
    message: string
    commitsBehind?: number
    commitsAhead?: number
  }> {
    const workspacePath = this.getWorkspacePath()

    if (!workspacePath) {
      throw new Error('Workspace path not available')
    }

    if (!(await this.canPerformFileOperations())) {
      throw new Error('Cannot access git: workspace not ready')
    }

    try {
      const result = await syncWithMain(workspacePath, baseBranch)

      this.logger.info('Workspace sync completed', {
        taskId: this.taskId,
        workspacePath,
        success: result.success,
        conflicts: result.conflicts,
        commitsBehind: result.commitsBehind,
        commitsAhead: result.commitsAhead,
      })

      return result
    } catch (error) {
      this.logger.error('Failed to sync workspace with main', error as Error, {
        taskId: this.taskId,
        workspacePath,
      })
      throw new Error(`Sync failed: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  /**
   * Get file tree with git change information
   * This combines file listing and git diff data in a single performant call
   */
  async getFileTreeWithChanges(rootPath = '.'): Promise<FileTreeWithChangesResponse> {
    if (!(await this.canPerformFileOperations()))
      throw new Error('Cannot list files: workspace not ready')

    const workspacePath = this.getWorkspacePath()

    if (!workspacePath) {
      throw new Error('Workspace path not available')
    }

    try {
      // Parallel execution for performance
      const [gitDiff, allFiles] = await Promise.all([
        this.getGitDiff(),
        this.getAllFiles(workspacePath, rootPath),
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
        }
        if (file.status === 'modified') {
          totalModified++
        }
      }

      // Build file tree with git change information
      const tree = this.buildFileTree(allFiles, gitFileMap, rootPath)

      const gitSummary = {
        totalFiles: gitDiff.files.length,
        totalAdditions: gitDiff.totalAdditions,
        totalDeletions: gitDiff.totalDeletions,
        totalModified,
        totalStaged,
        compareBase: gitDiff.compareBase,
        hasChanges: gitDiff.files.length > 0,
      }

      return {
        tree,
        gitSummary,
      }
    } catch (error) {
      this.logger.error('Failed to get file tree with changes', error as Error, {
        taskId: this.taskId,
      })
      throw new Error(
        `Failed to get file tree with changes: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Efficiently get all files in the workspace
   */
  private async getAllFiles(workspacePath: string, rootPath: string): Promise<string[]> {
    const fullPath = path.join(workspacePath, rootPath)
    const allFiles: string[] = []

    async function walkDir(dir: string, relativePath: string): Promise<void> {
      try {
        const entries = await fs.readdir(dir, { withFileTypes: true })

        for (const entry of entries) {
          // Skip hidden files, node_modules, and common ignore patterns
          if (
            entry.name.startsWith('.') ||
            entry.name === 'node_modules' ||
            entry.name === '__pycache__'
          ) {
            continue
          }

          const fullEntryPath = path.join(dir, entry.name)
          const relativeEntryPath = relativePath ? path.join(relativePath, entry.name) : entry.name

          if (entry.isDirectory()) {
            allFiles.push(`${relativeEntryPath}/`)
            await walkDir(fullEntryPath, relativeEntryPath)
          } else {
            allFiles.push(relativeEntryPath)
          }
        }
      } catch (_error) {
        // Continue on permission errors or other issues
        // Note: Cannot use this.logger here as it's inside a nested function
        // Just continue silently
      }
    }

    await walkDir(fullPath, rootPath === '.' ? '' : rootPath)
    return allFiles.sort()
  }

  /**
   * Build hierarchical file tree with git change information
   */
  private buildFileTree(
    files: string[],
    gitFileMap: Map<
      string,
      {
        status: 'added' | 'modified' | 'deleted' | 'renamed'
        additions: number
        deletions: number
        isStaged: boolean
      }
    >,
    rootPath: string,
  ): FileTreeNode[] {
    const tree: FileTreeNode[] = []
    const nodeMap = new Map<string, FileTreeNode>()

    // Create root node if needed
    if (rootPath !== '.' && rootPath !== '') {
      const rootNode: FileTreeNode = {
        name: path.basename(rootPath),
        path: rootPath,
        type: 'directory',
        isChanged: false,
        children: [],
      }
      tree.push(rootNode)
      nodeMap.set(rootPath, rootNode)
    }

    for (const filePath of files) {
      const isDirectory = filePath.endsWith('/')
      const cleanPath = isDirectory ? filePath.slice(0, -1) : filePath
      const pathParts = cleanPath.split('/').filter((part) => part.length > 0)

      // Build intermediate directories
      let currentPath = rootPath === '.' ? '' : rootPath
      let currentChildren = tree

      for (let i = 0; i < pathParts.length - 1; i++) {
        const part = pathParts[i]
        const partPath = currentPath ? path.join(currentPath, part) : part

        if (!nodeMap.has(partPath)) {
          const dirNode: FileTreeNode = {
            name: part,
            path: partPath,
            type: 'directory',
            isChanged: false,
            children: [],
          }
          nodeMap.set(partPath, dirNode)
          currentChildren.push(dirNode)
        }

        const dirNode = nodeMap.get(partPath)!
        currentChildren = dirNode.children!
        currentPath = partPath
      }

      // Add the file/directory node
      const lastPart = pathParts[pathParts.length - 1]
      const fullPath = currentPath ? path.join(currentPath, lastPart) : lastPart

      if (!nodeMap.has(fullPath)) {
        const gitInfo = gitFileMap.get(cleanPath)
        const node: FileTreeNode = {
          name: lastPart,
          path: fullPath,
          type: isDirectory ? 'directory' : 'file',
          isChanged: !!gitInfo,
          ...(gitInfo && {
            gitStatus: gitInfo.status,
            additions: gitInfo.additions,
            deletions: gitInfo.deletions,
            isStaged: gitInfo.isStaged,
          }),
          ...(isDirectory && { children: [] }),
        }

        nodeMap.set(fullPath, node)
        currentChildren.push(node)

        // Mark parent directories as changed if this file is changed
        if (gitInfo) {
          this.markParentDirectoriesAsChanged(nodeMap, fullPath)
        }
      }
    }

    // Sort children recursively
    this.sortTreeNodes(tree)

    return tree
  }

  /**
   * Mark parent directories as changed when a child file is changed
   */
  private markParentDirectoriesAsChanged(
    nodeMap: Map<string, FileTreeNode>,
    filePath: string,
  ): void {
    const parts = filePath.split('/')

    for (let i = parts.length - 2; i >= 0; i--) {
      const parentPath = parts.slice(0, i + 1).join('/')
      const parentNode = nodeMap.get(parentPath)

      if (parentNode && parentNode.type === 'directory') {
        parentNode.isChanged = true
      }
    }
  }

  /**
   * Sort tree nodes with directories first, then files, alphabetically
   */
  private sortTreeNodes(nodes: FileTreeNode[]): void {
    nodes.sort((a, b) => {
      // Directories first
      if (a.type === 'directory' && b.type === 'file') return -1
      if (a.type === 'file' && b.type === 'directory') return 1

      // Then alphabetically
      return a.name.localeCompare(b.name)
    })

    // Recursively sort children
    for (const node of nodes) {
      if (node.children) {
        this.sortTreeNodes(node.children)
      }
    }
  }
}
