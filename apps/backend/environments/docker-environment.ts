import { exec, execSync } from 'node:child_process'
import { existsSync, promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { CommandResult, RepositoryConfig, ServerStatus } from '@moatless/types'
import type { AgentOptions, BaseAgent } from '../agents/base-agent.js'
import { ProxyAgent } from '../agents/proxy-agent.js'
import type { SettingsService } from '../services/settings.service.js'
import { generateContainerName } from '../utils/docker-container-name.js'
import { createTaskLogger } from '../utils/logger.js'
import { findAvailablePort } from '../utils/port-allocation.js'
import { BaseEnvironment } from './base-environment.js'

const execAsync = promisify(exec)

export class DockerEnvironment extends BaseEnvironment {
  private image = ''
  private runningAgent?: BaseAgent
  private _createdAt = new Date()
  private _error?: string
  private workspacePath: string
  private workerPortMapping?: { containerPort: number; hostPort: number }
  private logger = createTaskLogger(this.taskId)

  constructor(
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath: string,
    private settingsService: SettingsService,
  ) {
    super(taskId, repositoryConfig)
    this.workspacePath = workspacePath
    this.logger = createTaskLogger(taskId)
  }

  getType(): 'docker' {
    return 'docker'
  }

  getStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    return this.getContainerStatus()
  }

  private getContainerStatus(): 'not_setup' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    // Always generate the container name deterministically
    const containerName = generateContainerName(this.repositoryConfig.name, this.taskId)

    // Check container status docker inspect
    try {
      const containerStatus = execSync(
        `docker inspect --format='{{.State.Status}}' "${containerName}"`,
        { encoding: 'utf8', timeout: 2000 },
      ).trim()

      if (!containerStatus) {
        return 'not_setup'
      }

      if (containerStatus === 'created') {
        return 'initializing'
      }

      if (containerStatus === 'running') {
        return 'ready'
      }

      if (containerStatus === 'removing') {
        return 'cleaning'
      }

      if (containerStatus === 'exited' || containerStatus === 'dead') {
        return 'error'
      }
      return 'not_setup'
    } catch (error: any) {
      // Container doesn't exist
      if (
        error?.stderr?.toString().includes('No such object') ||
        error?.message?.includes('No such object')
      ) {
        return 'not_setup'
      }
      // Other errors
      this.logger.debug('Error checking container status', {
        error: error?.message,
        stderr: error?.stderr?.toString(),
      })
      return 'error'
    }
  }

  getCreatedAt(): Date {
    return this._createdAt
  }

  getError(): string | undefined {
    return this._error
  }

  getWorkspacePath(): string {
    return this.workspacePath
  }

  getContainerName(): string {
    // Always generate the container name deterministically
    return generateContainerName(this.repositoryConfig.name, this.taskId)
  }

  getPort(): number | undefined {
    return this.workerPortMapping?.hostPort
  }

  getImage(): string {
    return this.image
  }

  async setup(_iterationNumber?: number): Promise<void> {
    const startTime = Date.now()
    const containerName = this.getContainerName()

    this.logger.info('Starting Docker environment setup', {
      taskId: this.taskId,
      workspacePath: this.workspacePath,
      repositoryName: this.repositoryConfig.name,
    })

    // Write repository configuration to a temporary file
    const tmpDir = path.join(process.env.TMPDIR || '/tmp', 'moatless-configs')
    await fs.mkdir(tmpDir, { recursive: true })
    const configPath = path.join(tmpDir, `repository-config-${this.taskId}.json`)

    try {
      await fs.writeFile(configPath, JSON.stringify(this.repositoryConfig, null, 2))
      this.logger.info('Wrote repository configuration to temp file', { configPath })
    } catch (error) {
      this.logger.error('Failed to write repository configuration', error as Error)
      throw error
    }

    // Find Claude config using settings service (optional for tests)
    const claudeSettings = this.settingsService.getClaudeSettings()

    // Determine Claude config directory
    let claudeConfigDir: string
    if (claudeSettings.configDir) {
      claudeConfigDir = claudeSettings.configDir
    } else if (process.env.CLAUDE_CONFIG_HOME) {
      claudeConfigDir = process.env.CLAUDE_CONFIG_HOME
    } else {
      claudeConfigDir = path.join(process.env.HOME || '', '.claude')
    }

    let claudeConfigPath: string | null = null
    let foundCredentialsPath: string | null = null

    // First try CLAUDE_CONFIG_HOME/.config.json, then fallback to USER_HOME/.claude.json
    const configJsonPath = path.join(claudeConfigDir, '.config.json')
    if (existsSync(configJsonPath)) {
      claudeConfigPath = configJsonPath
      this.logger.info('Found Claude config at CLAUDE_CONFIG_DIR/.config.json', {
        configJsonPath,
      })
    }

    if (!claudeConfigPath) {
      const claudeJsonPath = path.join(process.env.HOME || '', '.claude.json')
      if (existsSync(claudeJsonPath)) {
        claudeConfigPath = claudeJsonPath
        this.logger.info('Found Claude config at USER_HOME/.claude.json', { claudeJsonPath })
      }
    }

    // Check for credentials file - try both .credentials.json and .credentials
    const credentialsJsonPath = path.join(claudeConfigDir, '.credentials.json')
    const credentialsPath = path.join(claudeConfigDir, '.credentials')
    if (existsSync(credentialsJsonPath)) {
      foundCredentialsPath = credentialsJsonPath
      this.logger.info('Found Claude credentials at .credentials.json', { credentialsJsonPath })
    } else if (existsSync(credentialsPath)) {
      foundCredentialsPath = credentialsPath
      this.logger.info('Found Claude credentials at .credentials', { credentialsPath })
    }

    // Only warn if no Claude config is found (don't throw error for tests)
    if (!claudeConfigPath) {
      this.logger.warn('No Claude config found - Claude Code agent may not work properly', {
        searchedPaths: [configJsonPath, path.join(process.env.HOME || '', '.claude.json')],
      })
    }

    // Fail if no credentials found when Claude config exists
    if (!foundCredentialsPath && claudeConfigPath) {
      const errorMsg =
        'No Claude credentials found (.credentials or .credentials.json) - Claude Code agent will not work'
      this.logger.error(errorMsg, {
        searchedPaths: [credentialsJsonPath, credentialsPath],
      })
      throw new Error(errorMsg)
    }
    if (!foundCredentialsPath) {
      this.logger.warn('No Claude credentials found - Claude Code agent may not work properly', {
        searchedPaths: [credentialsJsonPath, credentialsPath],
      })
    }

    try {
      // Allocate port for worker
      const workerPort = await findAvailablePort('worker')

      this.workerPortMapping = { containerPort: 8001, hostPort: workerPort }

      this.image =
        this.repositoryConfig.codeExecution.dockerImage || 'moatless-vibe-universal:latest'

      this.logger.info('Creating Docker container', {
        containerName: containerName,
        image: this.image,
        workerPort,
      })

      await this.createOrReuseContainer()

      this.logger.info('Starting Docker container', {
        containerName: containerName,
      })

      await this.startContainer()

      // Copy repository configuration to container BEFORE waiting for worker
      // The worker needs this config to start successfully
      try {
        // Copy the config file (directory already exists in image)
        await execAsync(`docker cp "${configPath}" "${containerName}:/data/repository-config.json"`)
        this.logger.info('Repository configuration copied to container')

        // Clean up temp file
        await fs.unlink(configPath)
      } catch (error) {
        this.logger.error('Failed to copy repository configuration', error as Error)
        throw error // This is critical - worker cannot start without it
      }

      // Wait for the container and worker to be ready
      this.logger.info('Waiting for container to be ready', {
        containerName: containerName,
      })

      await this.waitForContainerReady()

      // Copy Claude config files if they exist
      if (claudeConfigPath) {
        this.logger.info('Copying Claude config to container', { claudeConfigPath })
        await execAsync(
          `docker cp "${claudeConfigPath}" "${containerName}:/home/ubuntu/.claude/.claude.json"`,
        )
      }

      if (foundCredentialsPath) {
        this.logger.info('Copying Claude credentials to container', { foundCredentialsPath })
        await execAsync(
          `docker cp "${foundCredentialsPath}" "${containerName}:/home/ubuntu/.claude/.credentials.json"`,
        )
      }

      // Fix ownership of entire .claude directory and its contents to ubuntu user (run as root)
      // Only run this if we copied at least one file
      if (claudeConfigPath || foundCredentialsPath) {
        await execAsync(
          `docker exec --user root "${containerName}" chown -R ubuntu:ubuntu /home/ubuntu/.claude`,
        )
        this.logger.info('Claude config copied and ownership fixed successfully')
      } else {
        this.logger.info('No Claude config files to copy - skipping Claude setup')
      }

      this.logger.info('DockerEnvironment setup completed - container is ready', {
        containerName: containerName,
        workerPort: this.workerPortMapping.hostPort,
        setupTimeMs: Date.now() - startTime,
      })
    } catch (error) {
      const failureTime = Date.now() - startTime
      this._error = error instanceof Error ? error.message : String(error)

      this.logger.error('DockerEnvironment setup failed', error as Error, {
        setupTimeMs: failureTime,
        containerName: containerName,
      })

      await this.cleanup()
      throw error
    }
  }

  private async createOrReuseContainer(): Promise<void> {
    const containerName = this.getContainerName()

    // Check if container already exists
    const existingContainer = await this.getExistingContainer()

    if (existingContainer) {
      this.logger.info('Found existing container', {
        containerName: containerName,
        status: existingContainer.status,
      })

      // If container is running, reuse it
      if (existingContainer.status.includes('Up')) {
        this.logger.info('Container is running, reusing it')
        return
      }

      // If container is stopped, remove it and create a new one
      this.logger.info('Container is stopped, removing it and creating new one')
      await this.removeExistingContainer(existingContainer.id)
    }

    // No existing container found or removed stopped container, create a new one
    this.logger.info('Creating new container')
    await this.createContainer()
  }

  private async getExistingContainer(): Promise<{ id: string; status: string } | null> {
    const containerName = this.getContainerName()

    try {
      // Use exact name match with ^/name$ pattern to avoid partial matches
      const { stdout } = await execAsync(
        `docker ps -a --filter "name=^/${containerName}$" --format "{{.ID}},{{.Status}}"`,
      )

      if (!stdout.trim()) {
        this.logger.debug('No existing container found with exact name match')
        return null
      }

      const lines = stdout.trim().split('\n')
      for (const line of lines) {
        const [id, status] = line.split(',')
        if (id && status) {
          this.logger.debug('Found existing container', {
            containerId: id.trim(),
            status: status.trim(),
          })
          return { id: id.trim(), status: status.trim() }
        }
      }

      return null
    } catch (error) {
      this.logger.debug('Error checking for existing container', { error })
      return null
    }
  }

  private async removeExistingContainer(containerId: string): Promise<void> {
    try {
      this.logger.info('Removing existing container', { containerId })

      // First try to stop the container if it's running
      try {
        await execAsync(`docker stop ${containerId}`)
        this.logger.debug('Container stopped', { containerId })
      } catch (_stopError) {
        // Container might already be stopped, continue with removal
        this.logger.debug('Container stop failed (may already be stopped)', { containerId })
      }

      // Remove the container
      await execAsync(`docker rm ${containerId}`)
      this.logger.info('Container removed successfully', { containerId })
    } catch (error) {
      this.logger.error('Failed to remove existing container', error as Error, { containerId })
      throw error
    }
  }

  private async createContainer(): Promise<void> {
    const containerName = this.getContainerName()
    let createCommand = `docker create --name "${containerName}"`

    // Add moatless labels for easy cleanup
    createCommand += ` --label "created-by=moatless"`
    createCommand += ` --label "moatless.task-id=${this.taskId}"`
    createCommand += ` --label "moatless.created-at=${new Date().toISOString()}"`

    // Use repository path as container workspace path
    const containerWorkspacePath = this.repositoryConfig.path

    // Set working directory to container workspace path
    createCommand += ` -w "${containerWorkspacePath}"`

    // Add volume mappings
    const dataDir = process.env.DATA_DIR || './data'
    createCommand += ` -v "${this.workspacePath}:${containerWorkspacePath}"`
    createCommand += ` -v "${dataDir}:/data"`

    // Add main repo .git directory mount for worktrees
    if (this.repositoryConfig.useGitWorktrees) {
      // The repository path is the main repository path when using worktrees
      const mainRepoPath = this.repositoryConfig.path
      createCommand += ` -v "${mainRepoPath}/.git:/main-repo-git"`
      createCommand += ` -e "MAIN_REPO_PATH=${mainRepoPath}"`
      createCommand += ` -e "USE_GIT_WORKTREES=true"`

      // Extract worktree name for git configuration
      const worktreeName = path.basename(this.workspacePath)
      createCommand += ` -e "GIT_WORKTREE_NAME=${worktreeName}"`
    }

    // Mount Claude config directory using settings service
    const containerClaudeConfigDir = '/home/ubuntu/.claude'
    const claudeSettings = this.settingsService.getClaudeSettings()

    // Determine host Claude config directory
    let hostClaudeConfigDir: string
    if (claudeSettings.configDir) {
      hostClaudeConfigDir = claudeSettings.configDir
    } else if (process.env.CLAUDE_CONFIG_DIR) {
      hostClaudeConfigDir = process.env.CLAUDE_CONFIG_DIR
    } else {
      hostClaudeConfigDir = path.join(process.env.HOME || '', '.claude')
    }

    // Mount projects directory if it exists
    const projectsDir = path.join(hostClaudeConfigDir, 'projects')
    if (existsSync(projectsDir)) {
      createCommand += ` -v "${projectsDir}:${containerClaudeConfigDir}/projects"`
    }

    if (!this.workerPortMapping) {
      throw new Error('Worker port not allocated')
    }

    // Add port mappings
    createCommand += ` -p ${this.workerPortMapping.hostPort}:${this.workerPortMapping.containerPort}`

    // Add environment variables
    const port = process.env.PORT || '8000'
    const apiUrl =
      process.env.WORKER_API_URL || process.env.API_URL || `http://host.docker.internal:${port}`
    const apiKey = process.env.WORKER_API_KEY || process.env.API_KEY || 'test-key'

    createCommand += ` -e "WORKER_MODE=true"`
    createCommand += ` -e "WORKER_API_URL=${apiUrl}"`
    createCommand += ` -e "WORKER_API_KEY=${apiKey}"`
    createCommand += ` -e "WORKER_TASK_ID=${this.taskId}"`
    createCommand += ` -e "WORKSPACE_DIR=${containerWorkspacePath}"`
    createCommand += ` -e "DATA_DIR=/data"`
    createCommand += ` -e "PORT=${this.workerPortMapping?.containerPort || 8001}"`
    createCommand += ` -e "LOG_LEVEL=info"`

    // Pass through Claude authentication environment variables
    if (process.env.CLAUDE_AUTH_DIR) {
      createCommand += ` -e "CLAUDE_AUTH_DIR=${process.env.CLAUDE_AUTH_DIR}"`
    }
    if (process.env.ANTHROPIC_API_KEY) {
      createCommand += ` -e "ANTHROPIC_API_KEY=${process.env.ANTHROPIC_API_KEY}"`
    }
    createCommand += ` -e "CLAUDE_CONFIG_DIR=${containerClaudeConfigDir}"`
    createCommand += ` -e "HOME=${containerWorkspacePath}"`

    if (process.env.CLAUDE_CODE_MOCK) {
      createCommand += ` -e "CLAUDE_CODE_MOCK=${process.env.CLAUDE_CODE_MOCK}"`
    }

    // Pass GitHub settings if configured
    const githubSettings = this.settingsService.getGitHubSettings()
    if (githubSettings.personalAccessToken) {
      createCommand += ` -e "GITHUB_TOKEN=${githubSettings.personalAccessToken}"`
    }
    if (githubSettings.userName) {
      createCommand += ` -e "GIT_USER_NAME=${githubSettings.userName}"`
    }
    if (githubSettings.userEmail) {
      createCommand += ` -e "GIT_USER_EMAIL=${githubSettings.userEmail}"`
    }

    // Add repository-specific environment variables
    const envVars = this.repositoryConfig.codeExecution.envVars || {}
    for (const [key, value] of Object.entries(envVars)) {
      createCommand += ` -e "${key}=${value}"`
    }

    // Pass setup commands to container
    const setupCommands = this.repositoryConfig.codeExecution.setupCommands || []
    if (setupCommands.length > 0) {
      // Encode setup commands as JSON and pass as environment variable
      const setupCommandsJson = JSON.stringify(setupCommands)
      createCommand += ` -e "SETUP_COMMANDS=${setupCommandsJson.replace(/"/g, '\\"')}"`
    }

    createCommand += ` ${this.image}`

    this.logger.debug('Creating container', { createCommand })

    try {
      const { stdout } = await execAsync(createCommand)
      const containerId = stdout.trim()
      this.logger.info('Container created', { containerName: containerName, containerId })
    } catch (error) {
      this.logger.error('Failed to create container', error as Error)
      throw error
    }
  }

  private async startContainer(): Promise<void> {
    const containerName = this.getContainerName()
    this.logger.info('Starting container', { containerName: containerName })

    try {
      // Check if container is already running
      const { stdout } = await execAsync(
        `docker ps --filter "name=^/${containerName}$" --format "{{.ID}}"`,
      )

      if (stdout.trim()) {
        this.logger.info('Container is already running', { containerName: containerName })
        return
      }

      // Container is not running, start it
      await execAsync(`docker start ${containerName}`)
      this.logger.info('Container started', { containerName: containerName })
    } catch (error) {
      this.logger.error('Failed to start container', error as Error)
      throw error
    }
  }

  /**
   * Ensures that the container is ready for command execution or agent creation.
   * If the container is not setup or in error state, it will attempt to set it up.
   * If setup is already in progress, it will wait for completion.
   */
  private async ensureContainerReady(): Promise<void> {
    const status = this.getStatus()

    // Already ready - fast path
    if (status === 'ready') {
      return
    }

    // Handle states that need action
    switch (status) {
      case 'not_setup':
        this.logger.info('Container not setup, starting setup process')
        await this.setup()
        break

      case 'error':
        this.logger.info('Container in error state, attempting recovery', {
          previousError: this._error,
          containerName: this.getContainerName(),
        })

        // Clean up the failed container
        await this.cleanup()

        // Reset error state
        this._error = undefined

        // Setup container
        await this.setup()
        break

      case 'initializing':
        // Already in progress, just need to wait
        this.logger.info('Container setup in progress, waiting for completion', { status })
        break

      case 'cleaning':
        // Container is being removed, wait for it to finish then check again
        this.logger.info('Container is being cleaned up, waiting for completion')
        await new Promise((resolve) => setTimeout(resolve, 2000))
        // Recursively check status again after cleanup
        return this.ensureContainerReady()

      default:
        throw new Error(`Unexpected container status: ${status}`)
    }

    // Wait for container to be ready (for all non-ready states except cleaning)
    await this.waitForContainerReady()
  }

  /**
   * Waits for the container to be ready for command execution or agent creation.
   */
  private async waitForContainerReady(): Promise<void> {
    const maxWaitTime = 180000 // 3 minute timeout for waiting
    const checkInterval = 2000 // 2 seconds
    let elapsed = 0

    const containerName = this.getContainerName()

    this.logger.debug('Waiting for container to become ready', {
      containerName: containerName,
      workerPort: this.workerPortMapping?.hostPort,
      maxWaitTime,
    })

    // First, ensure we have worker port mapping
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    while (elapsed < maxWaitTime) {
      const status = this.getStatus()

      if (status === 'ready') {
        this.logger.info('Container is ready, wait for worker readiness', {
          elapsedMs: elapsed,
          containerName: containerName,
          workerPort: this.workerPortMapping.hostPort,
        })

        while (elapsed < maxWaitTime) {
          try {
            const response = await fetch(
              `http://localhost:${this.workerPortMapping?.hostPort}/health`,
            )
            if (response.ok) {
              this.logger.info('Worker is ready', {
                elapsedMs: elapsed,
                workerPort: this.workerPortMapping?.hostPort,
              })
              return
            }
          } catch (_error) {
            // Worker not ready yet
            if (elapsed % 5000 === 0) {
              // Log every 5 seconds
              this.logger.debug('Worker not ready yet, continuing to wait', {
                elapsedMs: elapsed,
                remainingMs: maxWaitTime - elapsed,
              })
            }
          }

          await new Promise((resolve) => setTimeout(resolve, checkInterval))
          elapsed += checkInterval
        }

        throw new Error(`Worker did not become ready within timeout (${maxWaitTime}ms)`)
      }

      if (status === 'error') {
        const error = this.getError() || 'Container failed'
        throw new Error(error)
      }

      if (status === 'not_setup') {
        throw new Error('Container lost during wait - setup may have failed')
      }

      // Log progress every 10 seconds
      if (elapsed % 10000 === 0 && elapsed > 0) {
        this.logger.debug('Container still starting', {
          elapsedMs: elapsed,
          remainingMs: maxWaitTime - elapsed,
          containerName: containerName,
        })
      }

      await new Promise((resolve) => setTimeout(resolve, checkInterval))
      elapsed += checkInterval
    }

    throw new Error(`Container did not become ready imeout (${maxWaitTime}ms)`)
  }

  async runCommand(command: string, timeout = 30000): Promise<CommandResult> {
    // Ensure container is ready before executing command
    await this.ensureContainerReady()

    // Double-check we have worker port mapping
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    const containerName = this.getContainerName()

    this.logger.debug(`Running command via docker exec: ${command}`)

    const startTime = Date.now()

    try {
      // Build docker exec command with git environment variables if needed
      const containerWorkspacePath = this.repositoryConfig.path
      let dockerCommand = `docker exec -w "${containerWorkspacePath}"`

      // Add git environment variables if using worktrees
      if (this.repositoryConfig.useGitWorktrees) {
        const worktreeName = path.basename(this.workspacePath)
        dockerCommand += ` -e "GIT_DIR=/main-repo-git/worktrees/${worktreeName}"`
        dockerCommand += ` -e "GIT_WORK_TREE=${containerWorkspacePath}"`
      }

      dockerCommand += ` ${containerName} /bin/bash -c "${command.replace(/"/g, '\\"')}"`

      const { stdout, stderr } = await execAsync(dockerCommand, { timeout })

      return {
        exitCode: 0,
        output: stdout + stderr,
        durationMs: Date.now() - startTime,
        command,
      }
    } catch (error) {
      this.logger.error('Command execution failed', error as Error)

      // Extract exit code from error if available
      let exitCode = 1
      let output = ''

      if (error instanceof Error) {
        output = error.message
        // Check if error has exit code property (from child_process)
        if ('code' in error && typeof error.code === 'number') {
          exitCode = error.code
        }
        // Extract stdout/stderr from exec error
        if ('stdout' in error && typeof error.stdout === 'string') {
          output = error.stdout
        }
        if ('stderr' in error && typeof error.stderr === 'string') {
          output += error.stderr
        }
      }

      return {
        exitCode,
        output,
        durationMs: Date.now() - startTime,
        command,
      }
    }
  }

  async startServer(): Promise<ServerStatus> {
    // Ensure container is ready before starting server
    await this.ensureContainerReady()

    this.logger.info('Starting server via worker')

    // Get server configuration from repository config
    const serverCommand = this.repositoryConfig.previewConfig?.serverStartCommand
    if (!serverCommand) {
      throw new Error('No server command configured for this environment')
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/start`,
        {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
          },
          body: JSON.stringify({
            serverCommand,
            envVars: this.repositoryConfig.codeExecution.envVars || {},
          }),
        },
      )

      if (!response.ok) {
        throw new Error(`Failed to start server: ${response.statusText}`)
      }

      const result = (await response.json()) as ServerStatus

      // For proxy approach, we don't expose the direct port
      // The preview will be accessed through the proxy endpoint
      if (result.running) {
        result.url = undefined // Frontend will use proxy endpoint
      }

      return result
    } catch (error) {
      this.logger.error('Failed to start server', error as Error)
      throw error
    }
  }

  async stopServer(): Promise<boolean> {
    if (!this.workerPortMapping) {
      return false
    }

    this.logger.info('Stopping server via worker', {
      workerPort: this.workerPortMapping.hostPort,
      containerName: this.getContainerName(),
    })

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping.hostPort}/server/stop`,
        {
          method: 'POST',
          signal: AbortSignal.timeout(5000), // 5 second timeout
        },
      )

      if (!response.ok) {
        // Handle 404 gracefully - server may already be stopped
        if (response.status === 404) {
          this.logger.debug('Server stop endpoint not found - server may already be stopped')
          return true
        }
        throw new Error(`Failed to stop server: ${response.statusText}`)
      }

      const result = (await response.json()) as { success: boolean }
      return result.success
    } catch (error) {
      // Handle connection errors gracefully during shutdown
      if (error instanceof Error) {
        if (error.message.includes('ECONNREFUSED') || error.message.includes('ENOTFOUND')) {
          this.logger.debug('Worker container not reachable - treating as already stopped')
          return true
        }
      }
      this.logger.error('Failed to stop server', error as Error)
      return false
    }
  }

  async getServerStatus(): Promise<ServerStatus> {
    // Check if container is ready before getting server status
    const status = this.getStatus()
    if (status !== 'ready') {
      return {
        running: false,
        configured: !!this.repositoryConfig.previewConfig?.serverStartCommand,
        uptimeSeconds: 0,
      }
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/status`,
      )

      if (!response.ok) {
        throw new Error(`Failed to get server status: ${response.statusText}`)
      }

      const result = (await response.json()) as ServerStatus

      // For proxy approach, we don't expose the direct port
      // The preview will be accessed through the proxy endpoint
      if (result.running) {
        result.url = undefined // Frontend will use proxy endpoint
      }

      return result
    } catch (error) {
      this.logger.error('Failed to get server status', error as Error)
      return {
        running: false,
        configured: !!this.repositoryConfig.previewConfig?.serverStartCommand,
        uptimeSeconds: 0,
      }
    }
  }

  async getServerLogs(lines = 100): Promise<string> {
    // Check if container is ready before getting server logs
    const status = this.getStatus()
    if (status !== 'ready') {
      return 'Container not ready - no logs available'
    }

    try {
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/logs?lines=${lines}`,
      )

      if (!response.ok) {
        throw new Error(`Failed to get server logs: ${response.statusText}`)
      }

      const result = (await response.json()) as { logs: string }
      return result.logs
    } catch (error) {
      this.logger.error('Failed to get server logs', error as Error)
      return `Error getting logs: ${error}`
    }
  }

  getPreviewPort(): number | undefined {
    return undefined // Preview port managed by worker
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
    // Ensure container is ready before proxying request
    await this.ensureContainerReady()

    try {
      // Proxy the request through the worker which will forward to the preview server
      const response = await fetch(
        `http://localhost:${this.workerPortMapping!.hostPort}/server/proxy${request.path}`,
        {
          method: request.method,
          headers: request.headers,
          body: request.body,
        },
      )

      const responseHeaders = new Headers()
      response.headers.forEach((value, key) => {
        responseHeaders.set(key, value)
      })

      // Read response body
      const contentType = response.headers.get('content-type') || ''
      let body: string | Buffer

      if (
        contentType.includes('text/') ||
        contentType.includes('application/json') ||
        contentType.includes('application/javascript')
      ) {
        body = await response.text()
      } else {
        body = Buffer.from(await response.arrayBuffer())
      }

      return {
        status: response.status,
        headers: responseHeaders,
        body,
      }
    } catch (error) {
      this.logger.error('Failed to proxy preview request', error as Error)
      throw error
    }
  }

  async createAgent(agentType: string, options?: AgentOptions): Promise<BaseAgent> {
    // Ensure container is ready before creating agent
    await this.ensureContainerReady()

    // Double-check we have worker port mapping
    if (!this.workerPortMapping) {
      throw new Error('Worker port mapping not available - container setup incomplete')
    }

    // Return a proxy agent that delegates to the worker
    return new ProxyAgent({
      taskId: this.taskId,
      agentType,
      workerPort: this.workerPortMapping.hostPort,
      options,
      logger: this.logger,
    })
  }

  async stopAgent(): Promise<boolean> {
    if (!this.runningAgent) {
      return false
    }

    this.logger.info('Stopping running agent')

    try {
      // Make HTTP request to worker to stop agent
      const workerUrl = `http://localhost:${this.workerPortMapping?.hostPort}/stop-agent`

      const response = await fetch(workerUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taskId: this.taskId }),
      })

      if (!response.ok) {
        const error = await response.text()
        this.logger.error('Failed to stop agent in worker', new Error(error))
      }

      this.runningAgent = undefined
      return true
    } catch (error) {
      this.logger.error('Error stopping agent', error as Error)
      this.runningAgent = undefined
      return false
    }
  }

  isRunning(): boolean {
    return !!this.runningAgent
  }

  setRunningAgent(agent: BaseAgent): void {
    this.runningAgent = agent
  }

  getRunningAgent(): BaseAgent | undefined {
    return this.runningAgent
  }

  async teardown(force = false): Promise<void> {
    this.logger.info('Tearing down DockerEnvironment', {
      force,
      containerName: this.getContainerName(),
    })

    try {
      // Stop agent
      await this.stopAgent()

      // Stop server - don't let failures here stop the teardown
      try {
        await this.stopServer()
      } catch (serverError) {
        this.logger.warn('Error stopping server during teardown (continuing)', {
          error: (serverError as Error).message,
          stack: (serverError as Error).stack,
        })
      }

      // Cleanup container
      await this.cleanup()

      this.logger.info('DockerEnvironment teardown completed')
    } catch (error) {
      this.logger.error('Error during teardown', error as Error)
      if (!force) {
        throw error
      }
    }
  }

  private async cleanup(): Promise<void> {
    const containerName = this.getContainerName()

    // Check if container exists before trying to clean it up
    const status = this.getStatus()
    if (status === 'not_setup') {
      return
    }

    try {
      // Stop container
      await execAsync(`docker stop ${containerName}`)
      this.logger.info('Container stopped', { containerName: containerName })
    } catch (error) {
      this.logger.warn('Failed to stop container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    try {
      // Remove container
      await execAsync(`docker rm ${containerName}`)
      this.logger.info('Container removed', { containerName: containerName })
    } catch (error) {
      this.logger.warn('Failed to remove container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
}
