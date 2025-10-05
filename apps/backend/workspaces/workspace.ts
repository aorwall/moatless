import { promises as fs } from 'node:fs'
import path from 'node:path'
import type {
  CommandExecutionOptions,
  EnvironmentStatus,
  FileTreeNode,
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  RepositoryConfig,
  ServerStatus,
  ShellCommandLogEvent,
  ShellCommandResult,
  Workspace as WorkspaceInfo,
  WorkspaceStatusResponse,
} from '@moatless/types'
import type { BaseEnvironment } from '../environments/base-environment.js'
import type { DockerEnvironment } from '../environments/docker-environment.js'
import type { LocalEnvironment } from '../environments/local-environment.js'
import { FileOperations, type FileSearchRequest } from '../services/file-operations.service.js'
import type { BaseAgent } from '../services/index.js'
import { syncWithMain } from '../utils/index.js'
import { createTaskLogger } from '../utils/logger.js'
import { getTaskLogPath } from '../utils/task-log.js'

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
  private logFilePath: string

  // Properties that match the previous BaseWorkspace interface
  get id(): string {
    return this.taskId
  }

  get type(): 'local' | 'docker' {
    return this.environment.getType()
  }

  get status(): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
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
    // Use the currentAgent from the environment directly
    if (isLocalEnvironment(this.environment) || isDockerEnvironment(this.environment)) {
      const agentStatus = this.environment.getAgentStatus()
      return agentStatus.state === 'running' ? (this.environment as any).currentAgent : undefined
    }

    return undefined
  }

  constructor(
    private readonly taskId: string,
    private repositoryConfig: RepositoryConfig,
    private readonly environment: BaseEnvironment,
    workspacePath?: string,
    originalBranch?: string,
  ) {
    // Initialize logger
    this.logger = createTaskLogger(taskId)

    // Initialize log file path
    this.logFilePath = getTaskLogPath(taskId)

    // Initialize FileOperations with the workspace path from environment
    const initialWorkspacePath = workspacePath || environment.getWorkspacePath()
    this.fileOperations = new FileOperations(taskId, initialWorkspacePath, originalBranch)

    this.logger.debug('Workspace constructor', {
      type: this.type,
      repositoryId: repositoryConfig.id,
      repositoryPath: repositoryConfig.path,
      workspacePath: initialWorkspacePath,
      hasOriginalBranch: !!originalBranch,
      logFilePath: this.logFilePath,
    })
  }

  /**
   * Get the log file path for this workspace
   */
  getLogFilePath(): string {
    return this.logFilePath
  }

  /**
   * Set up the workspace (environment setup, then file operations validation)
   */
  async setup(): Promise<void> {
    this.logger.info('Starting Workspace setup', {
      type: this.type,
      repositoryId: this.repositoryConfig.id,
    })

    try {
      // Set up the environment first
      await this.environment.setup()

      const resolvedWorkspacePath = this.environment.getWorkspacePath()
      if (resolvedWorkspacePath) {
        this.fileOperations.setWorkspacePath(resolvedWorkspacePath)
      }

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

  async searchFiles(request: FileSearchRequest) {
    return this.fileOperations.searchFiles(request)
  }

  async getGitDiff() {
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

  async runCommand(command: string, timeout?: number): Promise<ShellCommandResult> {
    return this.environment.runCommand(command, timeout)
  }

  async *executeCommandsWithStream(
    options: CommandExecutionOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    const startTime = Date.now()
    let successCount = 0
    let failedCount = 0

    // Emit execution start event as system log
    yield {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Starting execution of ${options.commands.length} commands${options.purpose ? ` for ${options.purpose}` : ''}\n`,
    }

    for (let i = 0; i < options.commands.length; i++) {
      const command = options.commands[i]

      // Emit command start event
      yield {
        type: 'system',
        timestamp: new Date().toISOString(),
        text: `[${i + 1}/${options.commands.length}] Starting: ${command}\n`,
      }

      try {
        // Execute command with streaming and forward all events
        let commandFailed = false
        for await (const event of this.environment.runCommandWithStream(command, options.timeout)) {
          yield event

          // Check if command completed (system log with completion message)
          if (event.type === 'system' && event.text.includes('Command completed')) {
            successCount++
          } else if (event.type === 'system' && event.text.includes('Command failed')) {
            failedCount++
            commandFailed = true
          }
        }

        // If command failed and continueOnError is false, stop execution
        if (commandFailed && !options.continueOnError) {
          break
        }
      } catch (error) {
        failedCount++
        const errorMessage = error instanceof Error ? error.message : String(error)

        // Emit error event as system log
        yield {
          type: 'system',
          timestamp: new Date().toISOString(),
          text: `Error: ${errorMessage}\n`,
        }

        // If continueOnError is false, stop execution
        if (!options.continueOnError) {
          break
        }
      }
    }

    const totalDuration = Date.now() - startTime

    // Emit done event as system log with summary
    yield {
      type: 'system',
      timestamp: new Date().toISOString(),
      text: `Execution complete: ${successCount} succeeded, ${failedCount} failed out of ${options.commands.length} total (${totalDuration}ms)\n`,
    }
  }

  isRunning(): boolean {
    return this.environment.isRunning()
  }

  async stopAgent(): Promise<boolean> {
    const result = await this.environment.stopAgent()
    // Invalidate git status cache when agent stops (may have made changes)
    this.fileOperations.invalidateGitStatusCache()
    return result
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
        status: 'stopped',
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

  getStatus(): EnvironmentStatus {
    // Check if agent is running
    if (this.runningAgent) {
      return 'ready' // Running agents indicate workspace is ready
    }

    // Map environment status to workspace status
    const envStatus = this.environment.getStatus()
    switch (envStatus) {
      case 'not_created':
        return 'not_created'
      case 'initializing':
        return 'initializing'
      case 'ready':
        return 'ready'
      case 'error':
        return 'error'
      case 'cleaning':
        return 'cleaning'
      default:
        return 'not_created'
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

  getFileOperations(): FileOperations {
    return this.fileOperations
  }

  /**
   * Refresh the repository configuration without recreating the environment.
   * This stops the preview server, updates the config, and re-runs setup commands.
   * @param latestConfig The latest repository configuration
   */
  async refreshConfiguration(latestConfig: RepositoryConfig): Promise<void> {
    this.logger.info('Refreshing workspace configuration', {
      taskId: this.taskId,
      repositoryId: latestConfig.id,
    })

    // Update internal repository config reference
    this.repositoryConfig = latestConfig

    // Delegate to environment's refreshConfiguration
    await this.environment.refreshConfiguration(latestConfig)

    this.logger.info('Workspace configuration refreshed successfully')
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
      // Commands can only be executed when workspace is ready
      return status === 'ready'
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
  async getWorkspaceStatus(_backgroundTaskCount = 0): Promise<WorkspaceStatusResponse> {
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

    // Get agent state
    const agentState = await this.getAgentState()

    // Compute derived states
    const isRunning = agentState === 'running'
    const workspaceReady = environmentStatus === 'ready'
    const canExecute = workspaceReady && agentState === 'idle'

    return {
      // Workspace info
      workspaceState: await this.getWorkspaceExistence(),
      workspacePath: this.getWorkspacePath(),
      workspaceChangeStatus: this.getWorkspaceChangeStatus(gitStatus),

      // Environment info
      environmentType,
      environmentStatus,
      environmentError,
      environmentCreatedAt: environmentCreatedAt.toISOString(),

      // Agent info
      agentState,
      agentType: undefined, // Agent type will be set by calling code

      // Git status (already flat)
      gitStatus,

      // Capabilities
      capabilities: {
        fileOperations: fileOperationsAvailable,
        commandExecution: environmentStatus === 'ready',
        serverManagement: environmentStatus === 'ready',
      },

      // Computed states
      isRunning,
      workspaceReady,
      canExecute,

      // Config status
      configOutdated: false, // Will be set by WorkspaceService if needed

      // Logging
      logFilePath: this.logFilePath,
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

      // Invalidate git status cache after sync operation
      this.fileOperations.invalidateGitStatusCache()

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

  /**
   * Get workspace existence state
   */
  private async getWorkspaceExistence(): Promise<'exists' | 'missing' | 'invalid'> {
    try {
      const workspacePath = this.getWorkspacePath()
      const exists = await fs
        .access(workspacePath)
        .then(() => true)
        .catch(() => false)

      if (!exists) {
        return 'missing'
      }

      // Check if it's a valid git repository
      const gitDir = path.join(workspacePath, '.git')
      const isGitRepo = await fs
        .access(gitDir)
        .then(() => true)
        .catch(() => false)

      return isGitRepo ? 'exists' : 'invalid'
    } catch {
      return 'invalid'
    }
  }

  /**
   * Get workspace change status
   */
  private getWorkspaceChangeStatus(
    gitStatus: GitStatus,
  ): 'no_changes' | 'merged' | 'committed_unmerged' | 'uncommitted' | 'conflicts' {
    try {
      if (!gitStatus) {
        return 'no_changes'
      }

      // Check for conflicts
      if (gitStatus.hasConflicts) {
        return 'conflicts'
      }

      // Check for uncommitted changes
      if (gitStatus.hasUncommittedChanges) {
        return 'uncommitted'
      }

      // Check if we have commits ahead of main
      if (gitStatus.commitsAhead > 0) {
        return 'committed_unmerged'
      }

      // If we're on main and up to date
      if (gitStatus.commitsBehind === 0 && gitStatus.commitsAhead === 0) {
        return 'merged'
      }

      return 'no_changes'
    } catch {
      return 'no_changes'
    }
  }

  /**
   * Get agent state - for now this is a simple implementation
   * In the future, this could check if there are active agent processes
   */
  private async getAgentState(): Promise<'idle' | 'running' | 'none'> {
    const environment = this.getEnvironment()
    if (environment.getStatus() !== 'ready') {
      return 'none'
    }

    // Use the environment's agent status
    const agentStatus = environment.getAgentStatus()
    return agentStatus.state
  }
}
