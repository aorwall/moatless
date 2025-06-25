import { exec } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { Environment, LocalRepository } from '@shared/types/environment.js'
import { EventType } from '@shared/types/event.js'
import {
  type CommandResult,
  type ServerStatus,
  type Workspace,
  WorkspaceStatus,
} from '@shared/types/workspace.js'
import type { BaseAgent } from '../services/agents/base-agent.js'
import { ClaudeCodeAgent } from '../services/agents/claude-code-agent.js'
import type { EventService } from '../services/event.service.js'
import { searchFilesInDirectory } from '../utils/file-search.js'
import {
  getCurrentBranch,
  getGitDiffString,
  hasUncommittedChanges,
  isGitRepository,
} from '../utils/git-utils.js'
import { BaseWorkspace, type FileSearchRequest, type FileSearchResponse } from './base.js'

const execAsync = promisify(exec)

export class LocalWorkspace extends BaseWorkspace {
  private runningAgent?: BaseAgent
  private setupInProgress = false
  private setupComplete = false
  private workspacePath = ''
  private branch?: string
  private originalBranch?: string
  private branchName?: string
  private _createdAt = new Date()
  private _error?: string

  get worktreePath(): string {
    return this.workspacePath
  }

  get type(): 'local' {
    return 'local'
  }

  get status(): 'initializing' | 'ready' | 'error' | 'cleaning' {
    if (this._error) return 'error'
    if (this.setupInProgress) return 'initializing'
    if (this.setupComplete) return 'ready'
    return 'initializing'
  }

  get createdAt(): Date {
    return this._createdAt
  }

  get error(): string | undefined {
    return this._error
  }

  constructor(
    taskId: string,
    environment: Environment,
    eventService: EventService,
    originalBranch?: string,
    branchName?: string,
  ) {
    super(taskId, environment, eventService)
    this.originalBranch = originalBranch
    this.branchName = branchName
  }

  async setup(iterationNumber?: number): Promise<void> {
    this.setupInProgress = true
    try {
      // Emit setup started event
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: EventType.WORKSPACE,
        subType: 'setup_started',
        taskId: this.taskId,
        iterationId: iterationNumber,
        timestamp: new Date(),
        isError: false,
        payload: {
          environmentId: this.environment.id,
        },
      })

      // Check if repository is local
      if (this.environment.repository.type !== 'local') {
        throw new Error('LocalWorkspace requires local repository')
      }

      const repo = this.environment.repository as LocalRepository
      const workspaceDir = this.getWorktreeDir()

      // If worktrees are enabled, create specific workspace path
      if (repo.useGitWorktrees) {
        this.workspacePath = path.join(workspaceDir, this.getWorktreeFolderName())
      } else {
        this.workspacePath = workspaceDir
      }

      // Create workspace directory if it doesn't exist
      if (!(await this.pathExists(this.workspacePath))) {
        // Check if we should create a git worktree
        if (repo.useGitWorktrees && (await isGitRepository(repo.path))) {
          // Create git worktree
          await this.createGitWorktree(repo.path, this.workspacePath)

          await this.eventService.emitEvent({
            eventId: randomUUID(),
            type: EventType.WORKSPACE,
            subType: 'worktree_created',
            taskId: this.taskId,
            iterationId: iterationNumber,
            timestamp: new Date(),
            isError: false,
            payload: {
              path: this.workspacePath,
              branch: this.getBranchName(),
            },
          })
        } else {
          // Just create a regular directory
          await fs.mkdir(this.workspacePath, { recursive: true })

          await this.eventService.emitEvent({
            eventId: randomUUID(),
            type: EventType.WORKSPACE,
            subType: 'directory_created',
            taskId: this.taskId,
            iterationId: iterationNumber,
            timestamp: new Date(),
            isError: false,
            payload: {
              path: this.workspacePath,
            },
          })
        }
      }

      // Run setup commands if specified
      if (this.environment.codeExecution.setupCommands) {
        const commands = this.environment.codeExecution.setupCommands.split('\n')
        for (const command of commands) {
          const trimmedCommand = command.trim()
          if (!trimmedCommand) continue

          await this.eventService.emitEvent({
            eventId: randomUUID(),
            type: EventType.WORKSPACE,
            subType: 'command_running',
            taskId: this.taskId,
            iterationId: iterationNumber,
            timestamp: new Date(),
            isError: false,
            payload: { command: trimmedCommand },
          })

          try {
            const result = await this.runCommand(trimmedCommand)

            if (result.exitCode === 0) {
              await this.eventService.emitEvent({
                eventId: randomUUID(),
                type: EventType.WORKSPACE,
                subType: 'command_completed',
                taskId: this.taskId,
                iterationId: iterationNumber,
                timestamp: new Date(),
                isError: false,
                payload: {
                  command: trimmedCommand,
                  durationMs: result.durationMs,
                },
              })
            } else {
              await this.eventService.emitEvent({
                eventId: randomUUID(),
                type: EventType.WORKSPACE,
                subType: 'command_failed',
                taskId: this.taskId,
                iterationId: iterationNumber,
                timestamp: new Date(),
                isError: true,
                payload: {
                  command: trimmedCommand,
                  error: result.output,
                  exitCode: result.exitCode,
                },
              })
            }
          } catch (error) {
            await this.eventService.emitEvent({
              eventId: randomUUID(),
              type: EventType.WORKSPACE,
              subType: 'command_failed',
              taskId: this.taskId,
              iterationId: iterationNumber,
              timestamp: new Date(),
              isError: true,
              payload: {
                command: trimmedCommand,
                error: error instanceof Error ? error.message : String(error),
              },
            })
          }
        }
      }

      // TODO: Initialize and start server if configured

      // Emit setup completed event
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: EventType.WORKSPACE,
        subType: 'setup_completed',
        taskId: this.taskId,
        iterationId: iterationNumber,
        timestamp: new Date(),
        isError: false,
        payload: {
          workingDirectory: this.workspacePath,
        },
      })

      this.setupComplete = true
    } catch (error) {
      this._error = error instanceof Error ? error.message : String(error)
      throw error
    } finally {
      this.setupInProgress = false
    }
  }

  async teardown(force = false): Promise<void> {
    // Check for uncommitted changes first
    if (!force) {
      const hasChanges = await this.hasUncommittedChanges()
      if (hasChanges) {
        throw new Error('Workspace has uncommitted changes. Use force=true to close anyway.')
      }
    }

    // Stop the running agent if any
    if (this.runningAgent) {
      try {
        await this.runningAgent.stop()
      } catch (error) {
        console.error('Error stopping agent:', error)
      }
      this.runningAgent = undefined
    }

    // TODO: Stop the server if running

    // Reset setup state
    this.setupComplete = false

    // If we created a git worktree, remove it
    if (this.workspacePath && (await this.pathExists(this.workspacePath))) {
      const repo = this.environment.repository as LocalRepository
      if (repo.useGitWorktrees && this.originalBranch) {
        try {
          await this.removeGitWorktree(repo.path, this.workspacePath)
        } catch (error) {
          console.error('Error removing git worktree:', error)
          // Try manual cleanup as fallback
          await fs.rm(this.workspacePath, { recursive: true, force: true })
        }
      } else {
        // Just remove the directory
        await fs.rm(this.workspacePath, { recursive: true, force: true })
      }
    }
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
    }

    const searchPath = request.path || this.workspacePath
    const query = request.pattern || ''
    const searchResult = await searchFilesInDirectory(
      searchPath,
      query,
      100, // maxResults
      undefined, // filePattern
    )
    return searchResult
  }

  async getGitDiff(): Promise<string> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new Error(`Workspace path does not exist: ${this.workspacePath}`)
    }

    const [diffString] = await getGitDiffString(this.workspacePath, this.originalBranch)
    return diffString
  }

  async runCommand(command: string, timeout?: number): Promise<CommandResult> {
    const startTime = Date.now()

    try {
      const { stdout, stderr } = await execAsync(command, {
        cwd: this.workspacePath,
        timeout: timeout || 120000, // Default 2 minutes
      })

      // Combine stdout and stderr
      let output = stdout
      if (stderr) {
        output += output ? '\n' + stderr : stderr
      }

      return {
        command,
        output,
        exitCode: 0,
        durationMs: Date.now() - startTime,
      }
    } catch (error: any) {
      const output = error.stdout || ''
      const stderr = error.stderr || ''
      const combinedOutput = output + (stderr ? '\n' + stderr : '')

      return {
        command,
        output: combinedOutput || error.message,
        exitCode: error.code || -1,
        durationMs: Date.now() - startTime,
      }
    }
  }

  async triggerAgent(
    agentType: string,
    message: string,
    iterationNumber: number,
  ): Promise<BaseAgent> {
    // Create agent based on type
    let agent: BaseAgent

    if (agentType === 'claude-code' || agentType === 'claude_code') {
      // TODO: Get claude options from storage/config
      const options = {} // Placeholder for claude options

      agent = new ClaudeCodeAgent(this.taskId, options)
    } else {
      throw new Error(`Unknown agent type: ${agentType}`)
    }

    // Start the agent in the background
    this.runningAgent = agent
    this.runAgentAsync(agent, message, iterationNumber).catch((error) => {
      console.error(`Error running agent for task ${this.taskId}:`, error)
    })

    console.log(`Started ${agentType} agent for task ${this.taskId} in ${this.workspacePath}`)
    return agent
  }

  async start(agentType: string, message: string, iterationNumber: number): Promise<void> {
    if (this.getStatus() === WorkspaceStatus.RUNNING) {
      throw new Error(`Workspace ${this.taskId} is already running`)
    }

    // Run setup
    await this.setup(iterationNumber)

    // Trigger the agent
    await this.triggerAgent(agentType, message, iterationNumber)
  }

  isRunning(): boolean {
    return this.runningAgent !== undefined
  }

  getStatus(): WorkspaceStatus {
    // Check if agent is running
    if (this.runningAgent) {
      return WorkspaceStatus.RUNNING
    }

    // Check if setup is in progress
    if (this.setupInProgress) {
      return WorkspaceStatus.SETTING_UP
    }

    // Check if workspace directory exists
    if (this.workspacePath && this.setupComplete) {
      return WorkspaceStatus.IDLE
    }

    // Otherwise, workspace is not set up
    return WorkspaceStatus.NOT_SETUP
  }

  async hasUncommittedChanges(): Promise<boolean> {
    // If workspace doesn't exist or is not set up, no changes
    if (
      !this.workspacePath ||
      !(await this.pathExists(this.workspacePath)) ||
      this.getStatus() === WorkspaceStatus.NOT_SETUP
    ) {
      return false
    }

    return hasUncommittedChanges(this.workspacePath)
  }

  async mergeChanges(): Promise<string> {
    // Check if repository is local
    if (this.environment.repository.type !== 'local') {
      throw new Error('Merge only supported for local repositories')
    }

    const repo = this.environment.repository as LocalRepository

    // Check if worktrees are enabled
    if (!repo.useGitWorktrees) {
      throw new Error('Merge only supported for git worktree workspaces')
    }

    // Check for uncommitted changes
    if (await this.hasUncommittedChanges()) {
      throw new Error('Cannot merge with uncommitted changes')
    }

    // TODO: Implement merge logic
    throw new Error('Merge not yet implemented')
  }

  async stopAgent(): Promise<boolean> {
    if (!this.runningAgent) {
      console.log(`No agent running for task ${this.taskId}`)
      return false
    }

    console.log(`Stopping agent for task ${this.taskId}`)
    try {
      await this.runningAgent.stop()
      this.runningAgent = undefined
      console.log(`Agent stopped successfully for task ${this.taskId}`)
      return true
    } catch (error) {
      console.error(`Error stopping agent for task ${this.taskId}:`, error)
      this.runningAgent = undefined
      return false
    }
  }

  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    const fullPath = path.join(this.workspacePath, relativePath)

    try {
      const stats = await fs.stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const entries = await fs.readdir(fullPath, { withFileTypes: true })
      const files: string[] = []

      for (const entry of entries) {
        const entryPath = path.join(relativePath, entry.name)

        if (entry.isFile()) {
          files.push(entryPath)
        } else if (entry.isDirectory() && recursive && !entry.name.startsWith('.')) {
          const subFiles = await this.listFiles(entryPath, recursive)
          files.push(...subFiles)
        }
      }

      return files.sort()
    } catch (error) {
      throw new Error(
        `Failed to list files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  async readFile(filePath: string): Promise<string> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      throw new Error(
        `Failed to read file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')
    } catch (error) {
      throw new Error(
        `Failed to write file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  // Server methods (TODO: implement these)
  async startServer(): Promise<any> {
    throw new Error('Server not yet implemented')
  }

  async stopServer(): Promise<boolean> {
    return false
  }

  getServerStatus(): ServerStatus {
    return {
      running: false,
      configured: false,
      uptimeSeconds: 0,
    }
  }

  async getServerLogs(_lines = 100): Promise<string> {
    return 'No server configured or initialized'
  }

  toWorkspaceInfo(): Workspace {
    return {
      id: this.taskId,
      type: 'local' as const,
      environmentId: this.environment.id,
      status: this.status,
      createdAt: this.createdAt,
      path: this.workspacePath,
      error: this.error,
    }
  }

  // Private helper methods
  private async runAgentAsync(
    agent: BaseAgent,
    message: string,
    iterationNumber: number,
  ): Promise<void> {
    try {
      // Add the initial user message to the event stream
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: EventType.INPUT,
        taskId: this.taskId,
        iterationId: iterationNumber,
        timestamp: new Date(),
        isError: false,
        payload: { message },
      })

      // Run the agent with workspace directory
      const runOptions = {
        workspaceDir: this.workspacePath,
        iterationNumber,
      }

      for await (const event of agent.run(message, runOptions)) {
        // Save event to event stream
        await this.eventService.emitEvent(event)

        // Check if this is the final result message
        const msgType = event.payload?.type
        if (msgType === 'result' || event.type === EventType.RESULT) {
          break
        }
      }
    } catch (error) {
      console.error(`Error running agent for task ${this.taskId}:`, error)
      // Create error event
      await this.eventService.emitEvent({
        eventId: randomUUID(),
        type: EventType.ERROR,
        taskId: this.taskId,
        iterationId: iterationNumber,
        timestamp: new Date(),
        isError: true,
        payload: {
          error: error instanceof Error ? error.message : String(error),
          message: `Agent error: ${error instanceof Error ? error.message : String(error)}`,
        },
      })
    } finally {
      this.runningAgent = undefined
    }
  }

  private getWorktreeDir(): string {
    // Ensure we have a LocalRepository
    if (this.environment.repository.type !== 'local') {
      throw new Error('Workspace directory calculation requires local repository')
    }

    const repo = this.environment.repository as LocalRepository

    // If worktrees are disabled, use repository path directly
    if (!repo.useGitWorktrees) {
      return repo.path
    }

    // Check for environment variable first
    const worktreeDir = process.env.WORKTREE_DIR
    if (worktreeDir) {
      return worktreeDir
    }

    // Default to "../" relative to repo path
    return path.join(repo.path, '..')
  }

  private getBranchName(): string {
    return `workspace-${this.taskId}`
  }

  private getWorktreeFolderName(): string {
    // Get the last directory name from the repo path
    const repo = this.environment.repository as LocalRepository
    const repoLastDirName = path.basename(repo.path)

    // Use the branch name for the worktree folder
    return `${repoLastDirName}-${this.getBranchName()}`
  }

  private async createGitWorktree(repoPath: string, workspacePath: string): Promise<void> {
    // Get current branch to restore later
    this.originalBranch = await getCurrentBranch(repoPath)

    // Use provided branch name or create new branch name
    if (this.branchName) {
      // Create worktree from existing branch (no -b flag)
      this.branch = this.branchName
      await execAsync(`git worktree add "${workspacePath}" "${this.branch}"`, {
        cwd: repoPath,
        timeout: 30000,
      })
    } else {
      // Create new branch for worktree
      this.branch = this.getBranchName()
      await execAsync(`git worktree add -b "${this.branch}" "${workspacePath}"`, {
        cwd: repoPath,
        timeout: 30000,
      })
    }

    console.log(`Created worktree at ${workspacePath} with branch ${this.branch}`)
  }

  private async removeGitWorktree(repoPath: string, workspacePath: string): Promise<void> {
    try {
      // Remove worktree - always use --force to ensure removal
      await execAsync(`git worktree remove --force "${workspacePath}"`, {
        cwd: repoPath,
        timeout: 10000,
      })

      // Only try to delete the branch if we created it (no custom branch_name provided)
      if (!this.branchName && this.branch) {
        try {
          await execAsync(`git branch -D "${this.branch}"`, {
            cwd: repoPath,
            timeout: 5000,
          })
        } catch {
          // Branch deletion is best effort
        }
      }
    } catch (error) {
      console.error('Error removing worktree:', error)
      throw error
    }
  }

  private async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}
