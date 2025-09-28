import { exec, execSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { RepositoryConfig } from '@moatless/types'
import { generateContainerName } from '../utils/docker-container-name.js'
import { createTaskLogger } from '../utils/logger.js'
import { PortService } from './port.service.js'
import type { SettingsService } from './settings.service.js'

const execAsync = promisify(exec)

export interface ContainerInfo {
  id: string
  status: string
}

export interface PortMapping {
  containerPort: number
  hostPort: number
}

/**
 * Docker service for managing Docker containers and operations.
 * Provides centralized Docker functionality with proper dependency injection.
 */
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
    message?: string
  }> {
    const dockerRunning = await this.isDockerRunning()
    const hasClaudeToken = !!process.env.CLAUDE_CODE_OAUTH_TOKEN

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
    } catch (error: any) {
      // Container doesn't exist
      if (
        error?.stderr?.toString().includes('No such object') ||
        error?.message?.includes('No such object')
      ) {
        return 'not_created'
      }
      // Other errors
      this.logger.debug('Error checking container status', {
        error: error?.message,
        stderr: error?.stderr?.toString(),
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
    let createCommand = `docker create --name "${containerName}"`

    // Add moatless labels for easy cleanup
    createCommand += ` --label "created-by=moatless"`
    createCommand += ` --label "moatless.task-id=${taskId}"`
    createCommand += ` --label "moatless.created-at=${new Date().toISOString()}"`

    // Use repository path as container workspace path
    const containerWorkspacePath = repositoryConfig.path

    // Set working directory to container workspace path
    createCommand += ` -w "${containerWorkspacePath}"`

    // Add volume mappings
    const dataDir = process.env.DATA_DIR || './data'
    createCommand += ` -v "${workspacePath}:${containerWorkspacePath}"`
    createCommand += ` -v "${dataDir}:/data"`

    // Add main repo .git directory mount for worktrees
    if (repositoryConfig.useGitWorktrees) {
      // The repository path is the main repository path when using worktrees
      const mainRepoPath = repositoryConfig.path
      createCommand += ` -v "${mainRepoPath}/.git:/main-repo-git"`
      createCommand += ` -e "MAIN_REPO_PATH=${mainRepoPath}"`
      createCommand += ` -e "USE_GIT_WORKTREES=true"`

      // Extract worktree name for git configuration
      const worktreeName = path.basename(workspacePath)
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

    // Add port mappings
    createCommand += ` -p ${workerPortMapping.hostPort}:${workerPortMapping.containerPort}`

    // Add environment variables
    const port = process.env.PORT || '8000'
    const apiUrl =
      process.env.WORKER_API_URL || process.env.API_URL || `http://host.docker.internal:${port}`
    const apiKey = process.env.WORKER_API_KEY || process.env.API_KEY || 'test-key'

    createCommand += ` -e "WORKER_MODE=true"`
    createCommand += ` -e "WORKER_API_URL=${apiUrl}"`
    createCommand += ` -e "WORKER_API_KEY=${apiKey}"`
    createCommand += ` -e "WORKER_TASK_ID=${taskId}"`
    createCommand += ` -e "WORKSPACE_DIR=${containerWorkspacePath}"`
    createCommand += ` -e "DATA_DIR=/data"`
    createCommand += ` -e "PORT=${workerPortMapping?.containerPort || 8001}"`
    createCommand += ` -e "LOG_LEVEL=info"`

    // Pass through Claude authentication environment variables
    if (process.env.CLAUDE_AUTH_DIR) {
      createCommand += ` -e "CLAUDE_AUTH_DIR=${process.env.CLAUDE_AUTH_DIR}"`
    }
    if (process.env.ANTHROPIC_API_KEY) {
      createCommand += ` -e "ANTHROPIC_API_KEY=${process.env.ANTHROPIC_API_KEY}"`
    }

    // Pass Claude OAuth token if available
    if (claudeSettings.oauthToken) {
      createCommand += ` -e "CLAUDE_CODE_OAUTH_TOKEN=${claudeSettings.oauthToken}"`
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
    const envVars = repositoryConfig.codeExecution.envVars || {}
    for (const [key, value] of Object.entries(envVars)) {
      createCommand += ` -e "${key}=${value}"`
    }

    // Pass setup commands to container
    const setupCommands = repositoryConfig.codeExecution.setupCommands || []
    if (setupCommands.length > 0) {
      // Encode setup commands as JSON and pass as environment variable
      const setupCommandsJson = JSON.stringify(setupCommands)
      createCommand += ` -e "SETUP_COMMANDS=${setupCommandsJson.replace(/"/g, '\\"')}"`
    }

    createCommand += ` ${image}`

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

    // Pass through git user configuration to ensure commits use correct author
    const githubSettings = this.settingsService.getGitHubSettings()
    if (githubSettings.userName) {
      dockerCommand += ` -e "GIT_AUTHOR_NAME=${githubSettings.userName}"`
      dockerCommand += ` -e "GIT_COMMITTER_NAME=${githubSettings.userName}"`
    }
    if (githubSettings.userEmail) {
      dockerCommand += ` -e "GIT_AUTHOR_EMAIL=${githubSettings.userEmail}"`
      dockerCommand += ` -e "GIT_COMMITTER_EMAIL=${githubSettings.userEmail}"`
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
    const portService = new PortService(this.logger)
    const workerPort = await portService.findAvailablePort(8000)
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

/**
 * Factory function for creating DockerService following DI pattern
 */
export function createDockerService(settingsService: SettingsService): DockerService {
  return new DockerService(settingsService)
}
