import { ShellCommand } from '@moatless/shell-utils'
import type { GitChanges, GitStatus, ShellCommandLogEvent } from '@moatless/types'
import { injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import type { BaseAgent } from '../../agents/agent/base-agent'
import { ClaudeCodeAgent } from '../../claude/agent/claude-code-agent'
import type {
  AgentCreationOptions,
  BaseEnvironmentDriver,
  FileSearchRequest,
  FileSearchResult,
  RunCommandOptions,
} from './base-environment-driver.js'

/**
 * LocalEnvironmentDriver - Manages local execution of commands and agents
 *
 * This driver:
 * - Is stateless regarding workspace lifecycle (state lives in snapshots)
 * - Manages active agent instances via map (keyed by taskId)
 * - Executes commands via ShellCommand
 *
 * Note: Preview server management moved to PreviewRuntimeService
 * Note: Setup commands now executed by PreviewServer as part of command sequence
 */
@injectable()
export class LocalEnvironmentDriver implements BaseEnvironmentDriver {
  /**
   * Active agent instances keyed by taskId
   * Agents spawn child processes (claude CLI) that need lifecycle management
   */
  private activeAgents: Map<string, BaseAgent> = new Map()

  /**
   * Run a command and stream log events
   */
  async *runCommand(
    workspacePath: string,
    command: string,
    options?: RunCommandOptions,
  ): AsyncGenerator<ShellCommandLogEvent> {
    logger.info(
      {
        workspacePath,
        command,
        timeout: options?.timeout,
      },
      'Running command',
    )

    try {
      const shellCommand = new ShellCommand({
        workingDirectory: workspacePath,
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
          command,
          exitCode: shellCommand.getExitCode() || 0,
          duration: shellCommand.getDuration(),
        },
        'Command completed',
      )
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)

      logger.error({ err: error as Error, command, errorMessage }, 'Command failed')

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
  createAgent(taskId: string, agentType: string, options?: AgentCreationOptions): BaseAgent {
    logger.debug(
      {
        taskId,
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
    const agent = new ClaudeCodeAgent(taskId, agentOptions)
    this.activeAgents.set(taskId, agent)

    logger.info(
      {
        taskId,
        agentType,
        model: options?.model,
        hasApiKey: !!options?.apiKey,
      },
      'Agent created',
    )
    return agent
  }

  /**
   * Stop an agent instance
   */
  async stopAgent(taskId: string): Promise<void> {
    logger.debug({ taskId }, 'Stopping agent')

    const agent = this.activeAgents.get(taskId)
    if (!agent) {
      logger.debug({ taskId }, 'No active agent to stop')
      return
    }

    try {
      await agent.stop()
      this.activeAgents.delete(taskId)
      logger.info({ taskId }, 'Agent stopped')
    } catch (error) {
      logger.error({ err: error as Error, taskId }, 'Error stopping agent')
      // Force cleanup even on error
      this.activeAgents.delete(taskId)
      throw error
    }
  }

  /**
   * Read a file from the workspace
   */
  async readFile(workspacePath: string, filePath: string): Promise<string> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(workspacePath, filePath)

    try {
      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, workspacePath, filePath, errorMessage },
        'Failed to read file',
      )
      throw new Error(`Failed to read file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * Write a file to the workspace
   */
  async writeFile(workspacePath: string, filePath: string, content: string): Promise<void> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      // Ensure directory exists
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')

      logger.debug({ workspacePath, filePath }, 'File written successfully')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, workspacePath, filePath, errorMessage },
        'Failed to write file',
      )
      throw new Error(`Failed to write file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * List files in the workspace
   */
  async listFiles(workspacePath: string, relativePath = '.', recursive = false): Promise<string[]> {
    const { promises: fs } = await import('node:fs')
    const path = await import('node:path')

    const fullPath = path.join(workspacePath, relativePath)

    // Security check: ensure path is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(workspacePath)
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
  async searchFiles(workspacePath: string, request: FileSearchRequest): Promise<FileSearchResult> {
    const { searchFilesInDirectoryNew } = await import('../../../utils/index')

    try {
      const result = await searchFilesInDirectoryNew(workspacePath, request)

      return {
        files: result.results.map((r) => ({
          path: r.path,
          matches: 1,
        })),
        totalMatches: result.totalFound,
      }
    } catch (error) {
      logger.error(
        { err: error as Error, workspacePath, query: request.query },
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
  async getGitStatus(workspacePath: string): Promise<GitStatus> {
    const { promises: fs } = await import('node:fs')

    // Check if workspace exists
    try {
      await fs.access(workspacePath)
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
        getOriginSyncStatus(workspacePath),
        checkForConflicts(workspacePath),
        getGitDiffDetailed(workspacePath),
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
  async getGitChanges(workspacePath: string): Promise<GitChanges> {
    const { promises: fs } = await import('node:fs')

    // Check if workspace exists
    try {
      await fs.access(workspacePath)
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
      return getWorkingDirectoryChanges(workspacePath)
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
   * Cleanup - stop all active agents
   * Note: Preview server cleanup is now handled by PreviewRuntimeService
   */
  async cleanup(): Promise<void> {
    logger.info(
      {
        agentCount: this.activeAgents.size,
      },
      'Cleaning up all resources',
    )

    // Stop all agents
    const agentTasks = Array.from(this.activeAgents.keys())
    await Promise.all(agentTasks.map((taskId) => this.stopAgent(taskId)))
    this.activeAgents.clear()

    logger.info('Cleanup completed')
  }
}
