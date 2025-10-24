import { exec, execSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import { getPort } from '@moatless/shell-utils'
import type { RepositoryConfig } from '@moatless/types'
import { singleton } from 'tsyringe'
import { generateContainerName } from '../utils/docker-container-name.js'
import { createTaskLogger } from '../utils/logger.js'
import { SettingsService } from './settings.service.js'

const execAsync = promisify(exec)

interface ContainerInfo {
  id: string
  status: string
}

interface PortMapping {
  containerPort: number
  hostPort: number
}

/**
 * Docker service for managing Docker containers and operations.
 * Provides centralized Docker functionality with proper dependency injection.
 */
@singleton()
export class DockerService {
  private logger = createTaskLogger('DockerService')

  constructor(private settingsService: SettingsService) {}

  // ===============================
  // Docker Environment Detection
  // ===============================

  /**
   * Check if Docker is running
   */
  async isDockerRunning(): Promise<boolean> {
    try {
      await execAsync('docker info', { timeout: 5000 })
      return true
    } catch {
      return false
    }
  }

  /**
   * Get Docker version
   */
  async getDockerVersion(): Promise<string | null> {
    try {
      const { stdout } = await execAsync('docker --version', { timeout: 5000 })
      return stdout.trim()
    } catch {
      return null
    }
  }

  /**
   * Check if Docker is available and Claude OAuth token is set
   * This is specifically for running tests that require both Docker and Claude authentication
   */
  async checkDockerAvailable(): Promise<{
    available: boolean
    dockerRunning: boolean
    hasClaudeToken: boolean
    message: string | undefined
  }> {
    const dockerRunning = await this.isDockerRunning()
    const hasClaudeToken = !!process.env['CLAUDE_CODE_OAUTH_TOKEN']

    const available = dockerRunning && hasClaudeToken

    let message: string | undefined
    if (!dockerRunning) {
      message = 'Docker is not running'
    } else if (!hasClaudeToken) {
      message = 'CLAUDE_CODE_OAUTH_TOKEN environment variable is not set'
    }

    return {
      available,
      dockerRunning,
      hasClaudeToken,
      message,
    }
  }

  // ===============================
  // Container Lifecycle Management
  // ===============================

  /**
   * Get container status using Docker inspect
   */
  getContainerStatus(
    containerName: string,
  ): 'not_created' | 'initializing' | 'ready' | 'error' | 'cleaning' {
    try {
      const containerStatus = execSync(
        `docker inspect --format='{{.State.Status}}' "${containerName}"`,
        { encoding: 'utf8', timeout: 2000 },
      ).trim()

      if (!containerStatus) {
        return 'not_created'
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
      return 'not_created'
    } catch (error: unknown) {
      // Container doesn't exist
      const errorMessage = error instanceof Error ? error.message : String(error)
      const stderr =
        error && typeof error === 'object' && 'stderr' in error ? String(error.stderr) : undefined

      if (errorMessage.includes('No such object') || stderr?.includes('No such object')) {
        return 'not_created'
      }
      // Other errors
      this.logger.debug('Error checking container status', {
        error: errorMessage,
        stderr,
      })
      return 'error'
    }
  }

  /**
   * Get existing container information
   */
  async getExistingContainer(containerName: string): Promise<ContainerInfo | null> {
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

  /**
   * Remove existing container
   */
  async removeExistingContainer(containerId: string): Promise<void> {
    try {
      this.logger.info('Removing existing container', { containerId })

      // First try to stop the container if it's running
      try {
        await execAsync(`docker stop ${containerId}`)
        this.logger.debug('Container stopped', { containerId })
      } catch (_stopError) {
        // Container might already be stopped, continue with removal
        this.logger.debug('Container stop failed (may already be stopped)', {
          containerId,
        })
      }

      // Remove the container
      await execAsync(`docker rm ${containerId}`)
      this.logger.info('Container removed successfully', { containerId })
    } catch (error) {
      this.logger.error('Failed to remove existing container', error as Error, {
        containerId,
      })
      throw error
    }
  }

  /**
   * Create Docker container with full configuration
   */
  async createContainer(
    containerName: string,
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath: string,
    workerPortMapping: PortMapping,
    image: string,
  ): Promise<void> {
    const containerWorkspacePath = repositoryConfig.path
    let createCommand = this.buildBaseCommand(containerName, taskId, containerWorkspacePath)

    createCommand = this.addVolumeMappings(
      createCommand,
      workspacePath,
      containerWorkspacePath,
      repositoryConfig,
    )

    createCommand = this.addClaudeConfigMount(createCommand)
    createCommand = this.addPortMapping(createCommand, workerPortMapping)
    createCommand = this.addWorkerEnvironment(
      createCommand,
      taskId,
      containerWorkspacePath,
      workerPortMapping,
    )
    createCommand = this.addClaudeEnvironment(createCommand)
    createCommand = this.addRepositoryEnvironment(createCommand, repositoryConfig)

    createCommand += ` ${image}`

    await this.executeContainerCreate(containerName, createCommand)
  }

  private buildBaseCommand(
    containerName: string,
    taskId: string,
    containerWorkspacePath: string,
  ): string {
    let command = `docker create --name "${containerName}"`
    command += ` --label "created-by=moatless"`
    command += ` --label "moatless.task-id=${taskId}"`
    command += ` --label "moatless.created-at=${new Date().toISOString()}"`
    command += ` -w "${containerWorkspacePath}"`
    return command
  }

  private addVolumeMappings(
    command: string,
    workspacePath: string,
    containerWorkspacePath: string,
    repositoryConfig: RepositoryConfig,
  ): string {
    const dataDir = process.env['DATA_DIR'] || './data'
    let result = command
    result += ` -v "${workspacePath}:${containerWorkspacePath}"`
    result += ` -v "${dataDir}:/data"`

    if (repositoryConfig.useGitWorktrees) {
      const mainRepoPath = repositoryConfig.path
      const worktreeName = path.basename(workspacePath)
      result += ` -v "${mainRepoPath}/.git:/main-repo-git"`
      result += ` -e "MAIN_REPO_PATH=${mainRepoPath}"`
      result += ` -e "USE_GIT_WORKTREES=true"`
      result += ` -e "GIT_WORKTREE_NAME=${worktreeName}"`
    }

    return result
  }

  private addClaudeConfigMount(command: string): string {
    const containerClaudeConfigDir = '/home/ubuntu/.claude'
    const claudeSettings = this.settingsService.getClaudeSettings()
    const hostClaudeConfigDir = this.resolveClaudeConfigDir(claudeSettings)
    const projectsDir = path.join(hostClaudeConfigDir, 'projects')

    let result = command
    if (existsSync(projectsDir)) {
      result += ` -v "${projectsDir}:${containerClaudeConfigDir}/projects"`
    }

    return result
  }

  private resolveClaudeConfigDir(claudeSettings: { configDir: string | undefined }): string {
    if (claudeSettings.configDir) {
      return claudeSettings.configDir
    }
    if (process.env['CLAUDE_CONFIG_DIR']) {
      return process.env['CLAUDE_CONFIG_DIR']
    }
    return path.join(process.env['HOME'] || '', '.claude')
  }

  private addPortMapping(command: string, workerPortMapping: PortMapping): string {
    return `${command} -p ${workerPortMapping.hostPort}:${workerPortMapping.containerPort}`
  }

  private addWorkerEnvironment(
    command: string,
    taskId: string,
    containerWorkspacePath: string,
    workerPortMapping: PortMapping,
  ): string {
    const port = process.env['PORT'] || '8000'
    const apiUrl =
      process.env['WORKER_API_URL'] ||
      process.env['API_URL'] ||
      `http://host.docker.internal:${port}`
    const apiKey = process.env['WORKER_API_KEY'] || process.env['API_KEY'] || 'test-key'

    let result = command
    result += ` -e "WORKER_MODE=true"`
    result += ` -e "WORKER_API_URL=${apiUrl}"`
    result += ` -e "WORKER_API_KEY=${apiKey}"`
    result += ` -e "WORKER_TASK_ID=${taskId}"`
    result += ` -e "WORKSPACE_DIR=${containerWorkspacePath}"`
    result += ` -e "DATA_DIR=/data"`
    result += ` -e "PORT=${workerPortMapping?.containerPort || 8001}"`
    result += ` -e "LOG_LEVEL=info"`
    result += ` -e "HOME=${containerWorkspacePath}"`

    return result
  }

  private addClaudeEnvironment(command: string): string {
    const containerClaudeConfigDir = '/home/ubuntu/.claude'
    const claudeSettings = this.settingsService.getClaudeSettings()

    let result = command
    if (process.env['CLAUDE_AUTH_DIR']) {
      result += ` -e "CLAUDE_AUTH_DIR=${process.env['CLAUDE_AUTH_DIR']}"`
    }
    if (process.env['ANTHROPIC_API_KEY']) {
      result += ` -e "ANTHROPIC_API_KEY=${process.env['ANTHROPIC_API_KEY']}"`
    }
    if (claudeSettings.oauthToken) {
      result += ` -e "CLAUDE_CODE_OAUTH_TOKEN=${claudeSettings.oauthToken}"`
    }
    if (process.env['CLAUDE_CODE_MOCK']) {
      result += ` -e "CLAUDE_CODE_MOCK=${process.env['CLAUDE_CODE_MOCK']}"`
    }

    result += ` -e "CLAUDE_CONFIG_DIR=${containerClaudeConfigDir}"`

    return result
  }

  private addRepositoryEnvironment(command: string, repositoryConfig: RepositoryConfig): string {
    let result = command
    const envVars = repositoryConfig.codeExecution.envVars || {}
    for (const [key, value] of Object.entries(envVars)) {
      result += ` -e "${key}=${value}"`
    }

    const setupCommands = repositoryConfig.codeExecution.setupCommands || []
    if (setupCommands.length > 0) {
      const setupCommandsJson = JSON.stringify(setupCommands)
      result += ` -e "SETUP_COMMANDS=${setupCommandsJson.replace(/"/g, '\\"')}"`
    }

    return result
  }

  private async executeContainerCreate(
    containerName: string,
    createCommand: string,
  ): Promise<void> {
    this.logger.debug('Creating container', { createCommand })

    try {
      const { stdout } = await execAsync(createCommand)
      const containerId = stdout.trim()
      this.logger.info('Container created', { containerName, containerId })
    } catch (error) {
      this.logger.error('Failed to create container', error as Error)
      throw error
    }
  }

  /**
   * Start Docker container
   */
  async startContainer(containerName: string): Promise<void> {
    this.logger.info('Starting container', { containerName })

    try {
      // Check if container is already running
      const { stdout } = await execAsync(
        `docker ps --filter "name=^/${containerName}$" --format "{{.ID}}"`,
      )

      if (stdout.trim()) {
        this.logger.info('Container is already running', { containerName })
        return
      }

      // Container is not running, start it
      await execAsync(`docker start ${containerName}`)
      this.logger.info('Container started', { containerName })
    } catch (error) {
      this.logger.error('Failed to start container', error as Error)
      throw error
    }
  }

  /**
   * Stop Docker container
   */
  async stopContainer(containerName: string): Promise<void> {
    try {
      await execAsync(`docker stop ${containerName}`)
      this.logger.info('Container stopped', { containerName })
    } catch (error) {
      this.logger.warn('Failed to stop container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  /**
   * Remove Docker container
   */
  async removeContainer(containerName: string): Promise<void> {
    try {
      await execAsync(`docker rm ${containerName}`)
      this.logger.info('Container removed', { containerName })
    } catch (error) {
      this.logger.warn('Failed to remove container', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }

  // ===============================
  // Docker Command Execution
  // ===============================

  /**
   * Execute command in Docker container
   */
  async executeCommandInContainer(
    containerName: string,
    command: string,
    repositoryConfig: RepositoryConfig,
    workspacePath: string,
    timeout = 30000,
  ): Promise<{ stdout: string; stderr: string }> {
    const containerWorkspacePath = repositoryConfig.path
    let dockerCommand = `docker exec -w "${containerWorkspacePath}"`

    // Add git environment variables if using worktrees
    if (repositoryConfig.useGitWorktrees) {
      const worktreeName = path.basename(workspacePath)
      dockerCommand += ` -e "GIT_DIR=/main-repo-git/worktrees/${worktreeName}"`
      dockerCommand += ` -e "GIT_WORK_TREE=${containerWorkspacePath}"`
    }

    dockerCommand += ` ${containerName} /bin/bash -c "${command.replace(/"/g, '\\"')}"`

    const { stdout, stderr } = await execAsync(dockerCommand, { timeout })
    return { stdout, stderr }
  }

  // ===============================
  // Port Management
  // ===============================

  /**
   * Allocate port for worker
   */
  async allocateWorkerPort(): Promise<PortMapping> {
    const workerPort = await getPort()
    return { containerPort: 8001, hostPort: workerPort }
  }

  // ===============================
  // Container Name Generation
  // ===============================

  /**
   * Generate container name for task
   */
  generateContainerName(repositoryName: string, taskId: string): string {
    return generateContainerName(repositoryName, taskId)
  }

  // ===============================
  // Helper for Testing
  // ===============================

  /**
   * Helper to skip tests when Docker is not available
   * Usage: await skipIfNoDocker()
   */
  async skipIfNoDocker(): Promise<void> {
    const { available, message } = await this.checkDockerAvailable()
    if (!available) {
      throw new Error(`Skipping test: ${message}`)
    }
  }
}
