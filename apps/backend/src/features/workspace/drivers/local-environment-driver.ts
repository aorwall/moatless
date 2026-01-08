import { ShellCommand } from '@moatless/shell-utils'
import type {
  FileTreeWithChangesResponse,
  GitChanges,
  GitStatus,
  ShellCommandLogEvent,
} from '@moatless/types'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import { ClaudeCodeAgent } from '../../claude/agent/claude-code-agent'
import type { PreviewServerConfig as PreviewRuntimeConfig } from '../../preview/types/index.js'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
  FileWithChangesResult,
  GitDiffResult,
  PreviewServerConfig,
  PreviewServerLogEvent,
  PreviewServerStatus,
  RunCommandOptions,
} from './base-environment-driver.js'

/**
 * LocalEnvironmentDriver - Task-scoped driver for local execution
 *
 * Each instance is scoped to a single task with:
 * - taskId: The task this driver operates on
 * - workspacePath: Path to the workspace directory
 *
 * The driver:
 * - Executes commands via ShellCommand
 * - Manages a single agent instance for the task
 * - Performs file operations on local filesystem
 *
 * Note: Preview server management is handled by PreviewRuntimeService
 */
export class LocalEnvironmentDriver implements BaseEnvironmentDriver {
  /**
   * Active agent instance for this task (if any)
   */
  private activeAgent: BaseAgent | null = null

  constructor(
    readonly taskId: string,
    private readonly workspacePath: string,
  ) {}

  private async getPreviewRuntime() {
    if (process.env['WORKER_MODE'] === 'true') {
      throw new Error('Preview runtime is not available in worker mode')
    }

    const { appContainer } = await import('../../../container/container')
    if (!appContainer) {
      throw new Error('Preview runtime container not initialized')
    }

    const previewModule = await import('../../preview/services/preview-runtime.service')
    return appContainer.get(previewModule.PreviewRuntimeService)
  }

  /**
   * Run a command and stream log events
   */
  async *runCommand(
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    logger.info(
      {
        taskId: this.taskId,
        workspacePath: this.workspacePath,
        command,
        timeout: options?.timeout,
      },
      'Running command',
    )

    try {
      const shellCommand = new ShellCommand({
        workingDirectory: this.workspacePath,
        timeout: options?.timeout || 30000,
        ...(options?.env && { env: options.env }),
        ...(options?.logFile && { logFile: options.logFile }),
      })

      // execute() now returns a promise that resolves when process exits
      // We don't await it yet so we can stream logs concurrently
      const completionPromise = shellCommand.execute(command)

      // Stream logs from the shell command
      for await (const logEvent of shellCommand.streamLogs()) {
        yield logEvent
      }

      // Wait for process to complete
      await completionPromise

      logger.info(
        {
          taskId: this.taskId,
          command,
          exitCode: shellCommand.getExitCode() || 0,
          duration: shellCommand.getDuration(),
        },
        'Command completed',
      )
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      logger.error(
        { err: error as Error, taskId: this.taskId, command, errorMessage },
        'Command failed',
      )

      // Emit error as system log
      yield {
        type: 'system',
        timestamp: new Date().toISOString(),
        text: `Error: ${errorMessage}\n`,
      }
    }
  }

  /**
   * Create an agent instance
   */
  // biome-ignore lint/suspicious/useAwait: Interface requires Promise return for Kubernetes async resolution
  async createAgent(agentType: string, options?: AgentCreationOptions): Promise<BaseAgent> {
    logger.debug(
      {
        taskId: this.taskId,
        agentType,
        model: options?.model,
        hasApiKey: !!options?.apiKey,
      },
      'Creating agent',
    )

    if (agentType !== 'claude-code') {
      throw new Error(`Unsupported agent type: ${agentType}`)
    }

    const agentOptions = {
      ...(options?.model && { model: options.model }),
      ...(options?.apiKey && { apiKey: options.apiKey }),
    }
    const agent = new ClaudeCodeAgent(this.taskId, agentOptions)
    this.activeAgent = agent

    logger.info(
      {
        taskId: this.taskId,
        agentType,
        model: options?.model,
        hasApiKey: !!options?.apiKey,
      },
      'Agent created',
    )
    return agent
  }

  /**
   * Stop the agent instance
   */
  async stopAgent(): Promise<void> {
    logger.debug({ taskId: this.taskId }, 'Stopping agent')

    if (!this.activeAgent) {
      logger.debug({ taskId: this.taskId }, 'No active agent to stop')
      return
    }

    try {
      await this.activeAgent.stop()
      this.activeAgent = null
      logger.info({ taskId: this.taskId }, 'Agent stopped')
    } catch (error) {
      logger.error({ err: error as Error, taskId: this.taskId }, 'Error stopping agent')
      // Force cleanup even on error
      this.activeAgent = null
      throw error
    }
  }

  // =============================================================================
  // Preview Server Methods - Delegate to PreviewRuntimeService
  // =============================================================================

  async startPreviewServer(config: PreviewServerConfig): Promise<PreviewServerStatus> {
    const runtime = await this.getPreviewRuntime()

    const runtimeConfig: PreviewRuntimeConfig = {
      serverId: this.taskId,
      workspacePath: this.workspacePath,
      serverCommands: config.serverCommands,
      setupCommands: config.setupCommands,
      envVars: config.envVars,
      startupTimeoutSeconds: config.startupTimeoutSeconds,
      enableInspector: config.enableInspector,
    }

    await runtime.start(runtimeConfig)
    return await this.getPreviewServerStatus()
  }

  async stopPreviewServer(): Promise<boolean> {
    const runtime = await this.getPreviewRuntime()
    await runtime.stop(this.taskId)
    return true
  }

  async getPreviewServerStatus(): Promise<PreviewServerStatus> {
    const runtime = await this.getPreviewRuntime()
    const status = await runtime.getStatus(this.taskId)
    const internalUrl = runtime.getInternalUrl(this.taskId)
    const port = runtime.getPort(this.taskId)

    return {
      status: status.status,
      internalUrl,
      externalUrl: status.url,
      port,
      pid: status.pid,
      uptimeSeconds: status.uptimeSeconds,
      error: status.error,
      logs: status.logs,
    }
  }

  async getPreviewServerLogs(lines = 100): Promise<string> {
    const runtime = await this.getPreviewRuntime()
    return await runtime.getLogs(this.taskId, lines)
  }

  async *streamPreviewServerLogs(_lines = 100): AsyncGenerator<PreviewServerLogEvent> {
    const runtime = await this.getPreviewRuntime()
    yield* runtime.streamLogs(this.taskId)
  }

  /**
   * Read a file from the workspace
   */
  async readFile(filePath: string): Promise<string> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(this.workspacePath, filePath)

    try {
      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, taskId: this.taskId, filePath, errorMessage },
        'Failed to read file',
      )
      throw new Error(`Failed to read file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * Write a file to the workspace
   */
  async writeFile(filePath: string, content: string): Promise<void> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      // Ensure directory exists
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')

      logger.debug({ taskId: this.taskId, filePath }, 'File written successfully')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, taskId: this.taskId, filePath, errorMessage },
        'Failed to write file',
      )
      throw new Error(`Failed to write file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * List files in the workspace
   */
  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(this.workspacePath, relativePath)

    // Security check: ensure path is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: Path outside workspace')
    }

    try {
      const stats = await fs.stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const files: string[] = []

      async function walkDir(dir: string, basePath: string): Promise<void> {
        const entries = await fs.readdir(dir, { withFileTypes: true })

        for (const entry of entries) {
          const entryFullPath = path.join(dir, entry.name)
          const entryRelativePath = path.join(basePath, entry.name)

          // Skip hidden files and common ignore patterns
          if (entry.name.startsWith('.') || entry.name === 'node_modules') {
            continue
          }

          if (entry.isDirectory()) {
            files.push(`${entryRelativePath}/`)
            if (recursive) {
              await walkDir(entryFullPath, entryRelativePath)
            }
          } else {
            files.push(entryRelativePath)
          }
        }
      }

      await walkDir(fullPath, relativePath === '.' ? '' : relativePath)
      return files.sort()
    } catch (error) {
      if ((error as { code?: string }).code === 'ENOENT') {
        throw new Error(`Path not found: ${relativePath}`)
      }
      throw new Error(
        `Failed to list files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Search for files in the workspace
   */
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    const { searchFilesInDirectoryNew } = await import('../../../utils/index')

    try {
      const result = await searchFilesInDirectoryNew(this.workspacePath, request)

      return {
        files: result.results.map((r) => ({
          path: r.path,
          matches: 1,
        })),
        totalMatches: result.totalFound,
      }
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.taskId, query: request.query },
        'Failed to search files',
      )
      throw new Error(
        `Failed to search files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Get git status for the workspace
   */
  async getGitStatus(): Promise<GitStatus> {
    const { promises: fs } = await import('node:fs')

    // Check if workspace exists
    try {
      await fs.access(this.workspacePath)
    } catch {
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      const { getOriginSyncStatus, checkForConflicts, getGitDiffDetailed } = await import(
        '../../../utils/git-utils.js'
      )

      const [syncStatus, hasConflicts, gitDiff] = await Promise.all([
        getOriginSyncStatus(this.workspacePath),
        checkForConflicts(this.workspacePath),
        getGitDiffDetailed(this.workspacePath),
      ])

      let additions = 0
      let deletions = 0
      let modifications = 0

      for (const file of gitDiff.files) {
        if (file.status === 'added') {
          additions++
        } else if (file.status === 'deleted') {
          deletions++
        } else if (file.status === 'modified') {
          modifications++
        }
      }

      return {
        currentBranch: syncStatus.currentBranch,
        baseBranch: syncStatus.baseBranch,
        hasUncommittedChanges: syncStatus.hasUncommittedChanges,
        hasConflicts,
        status: syncStatus.status,
        commitsAhead: syncStatus.commitsAhead,
        commitsBehind: syncStatus.commitsBehind,
        originExists: syncStatus.originExists,
        additions,
        deletions,
        modifications,
      }
    } catch {
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  /**
   * Get git changes for the workspace
   */
  async getGitChanges(): Promise<GitChanges> {
    const { promises: fs } = await import('node:fs')

    // Check if workspace exists
    try {
      await fs.access(this.workspacePath)
    } catch {
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      const { getWorkingDirectoryChanges } = await import('../../../utils/git-utils.js')
      return getWorkingDirectoryChanges(this.workspacePath)
    } catch {
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  /**
   * Get the workspace path for this driver
   */
  getWorkspacePath(): string {
    return this.workspacePath
  }

  /**
   * Push changes to remote repository
   */
  async pushChanges(options: {
    branch: string
    useUpstream?: boolean
  }): Promise<{ pushed: boolean; hasCommits: boolean; error?: string }> {
    const { pushGitCheckpoint, pushGitCheckpointWithUpstream } = await import(
      '../../../utils/git-utils.js'
    )

    try {
      if (options.useUpstream) {
        return await pushGitCheckpointWithUpstream(this.workspacePath, options.branch)
      }
      return await pushGitCheckpoint(this.workspacePath, options.branch)
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.taskId, branch: options.branch },
        'Failed to push changes',
      )
      return {
        pushed: false,
        hasCommits: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Create a git checkpoint (commit) for uncommitted changes
   */
  async createGitCheckpoint(
    message: string,
  ): Promise<{ commitHash: string | null; hasChanges: boolean; error?: string }> {
    const { createGitCheckpoint, hasUncommittedChanges } = await import(
      '../../../utils/git-utils.js'
    )

    try {
      // Check if there are changes to commit
      const hasChanges = await hasUncommittedChanges(this.workspacePath)
      if (!hasChanges) {
        return { commitHash: null, hasChanges: false }
      }

      // Create checkpoint commit
      const result = await createGitCheckpoint(this.workspacePath, message)
      return {
        commitHash: result.commitHash,
        hasChanges: true,
      }
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.taskId, message },
        'Failed to create git checkpoint',
      )
      return {
        commitHash: null,
        hasChanges: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      }
    }
  }

  /**
   * Get file tree with git change information
   */
  async getFileTreeWithChanges(rootPath = '.'): Promise<FileTreeWithChangesResponse> {
    const { getGitDiffDetailed } = await import('../../../utils/git-utils.js')
    const { buildFileTree } = await import('../utils/file-tree-builder.js')

    // Get all files recursively
    const allFiles = await this.listFiles(rootPath, true)

    // Get git diff information
    const gitDiff = await getGitDiffDetailed(this.workspacePath)

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

  /**
   * Get git diff for the workspace
   */
  async getGitDiff(): Promise<GitDiffResult> {
    const { getGitDiffDetailed } = await import('../../../utils/git-utils.js')

    const result = await getGitDiffDetailed(this.workspacePath)

    const resultObj: GitDiffResult = {
      files: result.files.map((f) => ({
        oldFile: f.oldFilename || f.filename,
        newFile: f.filename,
        hunks: [] as string[], // Simplified - could parse hunks from diffString if needed
      })),
    }
    if (result.diffString) {
      resultObj.diff = result.diffString
    }
    return resultObj
  }

  /**
   * Read a file with git change information
   */
  async readFileWithChanges(filePath: string): Promise<FileWithChangesResult> {
    // First read the current content
    const content = await this.readFile(filePath)

    try {
      // Get the file change information
      const { getFileChangeInfo } = await import('../../../utils/git-utils.js')
      const changeInfo = await getFileChangeInfo(this.workspacePath, filePath)

      const result: FileWithChangesResult = { content }

      if (changeInfo.oldContent) {
        result.oldContent = changeInfo.oldContent
      }
      if (changeInfo.additions !== undefined) {
        result.additions = changeInfo.additions
      }
      if (changeInfo.deletions !== undefined) {
        result.deletions = changeInfo.deletions
      }
      if (changeInfo.isModified !== undefined) {
        result.isModified = changeInfo.isModified
      }

      return result
    } catch (error) {
      // If we can't get git info, return default values for change info
      logger.debug(
        {
          taskId: this.taskId,
          filePath,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to get file change info',
      )

      return {
        content,
        additions: 0,
        deletions: 0,
        isModified: false,
      }
    }
  }

  /**
   * Get the current commit hash
   */
  async getCurrentCommitHash(): Promise<string | null> {
    const { promises: fs } = await import('node:fs')

    // Check if workspace exists
    try {
      await fs.access(this.workspacePath)
    } catch {
      return null
    }

    try {
      const { getCommitHash } = await import('../../../utils/git-utils.js')
      return await getCommitHash(this.workspacePath, true) // Use short hash
    } catch (error) {
      logger.debug(
        {
          taskId: this.taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to get commit hash',
      )
      return null
    }
  }
}
