import { exec, execSync } from 'node:child_process'
import { existsSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import {
  SANDBOX_CLAUDE_CONFIG_DIR,
  SANDBOX_DATA_DIR,
  SANDBOX_HOME,
} from '@moatless/sandbox-schemas'
import { getPort } from '@moatless/shell-utils'
import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { generateContainerName } from '../utils/docker-container-name'
import { logger } from '../utils/logger'
import { SettingsService } from './settings.service'

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
 *
 * Used by DockerWorkspaceProvisioner for container lifecycle management
 * and for system status checks (Docker availability).
 */
@injectable()
export class DockerService {
  private logger = logger.child({ service: 'DockerService' })

  constructor(@inject(SettingsService) private settingsService: SettingsService) {}

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
      this.logger.debug(
        {
          error: errorMessage,
          stderr,
        },
        'Error checking container status',
      )
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
          this.logger.debug(
            {
              containerId: id.trim(),
              status: status.trim(),
            },
            'Found existing container',
          )
          return { id: id.trim(), status: status.trim() }
        }
      }

      return null
    } catch (error) {
      this.logger.debug({ error }, 'Error checking for existing container')
      return null
    }
  }

  /**
   * Remove existing container
   */
  async removeExistingContainer(containerId: string): Promise<void> {
    try {
      this.logger.info({ containerId }, 'Removing existing container')

      // First try to stop the container if it's running
      try {
        await execAsync(`docker stop ${containerId}`)
        this.logger.debug({ containerId }, 'Container stopped')
      } catch (_stopError) {
        // Container might already be stopped, continue with removal
        this.logger.debug(
          {
            containerId,
          },
          'Container stop failed (may already be stopped)',
        )
      }

      // Remove the container
      await execAsync(`docker rm ${containerId}`)
      this.logger.info({ containerId }, 'Container removed successfully')
    } catch (error) {
      this.logger.error({ err: error as Error, containerId }, 'Failed to remove existing container')
      throw error
    }
  }

  /**
   * Create Docker container with full configuration
   *
   * @param containerName - Name of the container
   * @param taskId - Task ID for labeling and environment
   * @param repositoryConfig - Repository configuration
   * @param workspacePath - Path to workspace inside container
   * @param previewPortMapping - Optional port mapping for preview server (container:3000 -> host)
   * @param image - Docker image to use
   */
  async createContainer(
    containerName: string,
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath: string,
    previewPortMapping: PortMapping | undefined,
    image: string,
  ): Promise<void> {
    // All repositories use workspace path (cloned from remote)
    const containerWorkspacePath = workspacePath
    let createCommand = this.buildBaseCommand(containerName, taskId, containerWorkspacePath)

    createCommand = this.addVolumeMappings(
      createCommand,
      taskId,
      workspacePath,
      containerWorkspacePath,
      repositoryConfig,
    )

    createCommand = this.addClaudeConfigMount(createCommand)
    if (previewPortMapping) {
      createCommand = this.addPortMapping(createCommand, previewPortMapping)
    }
    createCommand = this.addWorkerEnvironment(createCommand, taskId, containerWorkspacePath)
    createCommand = this.addClaudeEnvironment(createCommand)
    createCommand = this.addRepositoryEnvironment(createCommand, repositoryConfig)

    createCommand += ` ${image}`

    await this.executeContainerCreate(containerName, createCommand)
  }

  private buildBaseCommand(
    containerName: string,
    taskId: string,
    _containerWorkspacePath: string,
  ): string {
    let command = `docker create --name "${containerName}"`
    // Attach to moatless network for container-to-container communication
    // Network name can be configured via DOCKER_NETWORK env var (docker-compose adds a prefix)
    const dockerNetwork = process.env['DOCKER_NETWORK'] || 'moatless'
    command += ` --network ${dockerNetwork}`
    command += ` --label "created-by=moatless"`
    command += ` --label "moatless.task-id=${taskId}"`
    command += ` --label "moatless.created-at=${new Date().toISOString()}"`
    // Note: Do NOT use -w flag, as it causes Docker to create the directory as root
    // The workspace directory is created by the entrypoint script with proper ownership
    return command
  }

  private addVolumeMappings(
    command: string,
    taskId: string,
    _workspacePath: string,
    _containerWorkspacePath: string,
    _repositoryConfig: RepositoryConfig,
  ): string {
    const baseDataDir = process.env['DATA_DIR'] || './data'

    // Create per-task directory for isolated storage (similar to Kubernetes PVCs)
    const taskDir = path.join(baseDataDir, 'tasks', taskId)

    // Ensure task directory exists
    if (!existsSync(taskDir)) {
      mkdirSync(taskDir, { recursive: true })
    }

    let result = command
    // Note: We don't mount a workspace volume because the container clones
    // the repository internally using GIT_CLONE_URL environment variable.
    // The workspace lives entirely inside the container.
    // Mount task-specific directory for isolated storage per task
    result += ` -v "${taskDir}:/data"`

    return result
  }

  private addClaudeConfigMount(command: string): string {
    // Match the CLAUDE_CONFIG_DIR set in Dockerfile.sandbox-base
    const containerClaudeConfigDir = SANDBOX_CLAUDE_CONFIG_DIR
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
    result += ` -e "DATA_DIR=${SANDBOX_DATA_DIR}"`
    result += ` -e "PORT=8001"` // Sandbox API always on port 8001
    result += ` -e "LOG_LEVEL=info"`
    // Set HOME to the moatless user's home directory, not the workspace
    // The workspace must be empty for git clone to succeed
    result += ` -e "HOME=${SANDBOX_HOME}"`

    // Redis configuration for event publishing
    // The sandbox container is on the same Docker network as Redis,
    // so it can resolve the 'redis' hostname from REDIS_URL
    const redisUrl = process.env['REDIS_URL']
    if (redisUrl) {
      result += ` -e "REDIS_URL=${redisUrl}"`
    }

    return result
  }

  private addClaudeEnvironment(command: string): string {
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

    // CLAUDE_CONFIG_DIR and CLAUDE_BIN_DIR are now set in Dockerfile.sandbox-base
    // No need to pass them as runtime environment variables

    return result
  }

  private addRepositoryEnvironment(command: string, repositoryConfig: RepositoryConfig): string {
    let result = command

    // Pass git clone URL so the sandbox can clone the repository internally
    if (repositoryConfig.remote?.url) {
      result += ` -e "GIT_CLONE_URL=${repositoryConfig.remote.url}"`
    }

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
    this.logger.debug({ createCommand }, 'Creating container')

    try {
      const { stdout } = await execAsync(createCommand)
      const containerId = stdout.trim()
      this.logger.info({ containerName, containerId }, 'Container created')
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to create container')
      throw error
    }
  }

  /**
   * Start Docker container
   */
  async startContainer(containerName: string): Promise<void> {
    this.logger.info({ containerName }, 'Starting container')

    try {
      // Check if container is already running
      const { stdout } = await execAsync(
        `docker ps --filter "name=^/${containerName}$" --format "{{.ID}}"`,
      )

      if (stdout.trim()) {
        this.logger.info({ containerName }, 'Container is already running')
        return
      }

      // Container is not running, start it
      await execAsync(`docker start ${containerName}`)
      this.logger.info({ containerName }, 'Container started')
    } catch (error) {
      this.logger.error({ err: error as Error }, 'Failed to start container')
      throw error
    }
  }

  /**
   * Stop Docker container
   */
  async stopContainer(containerName: string): Promise<void> {
    try {
      await execAsync(`docker stop ${containerName}`)
      this.logger.info({ containerName }, 'Container stopped')
    } catch (error) {
      this.logger.warn(
        {
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to stop container',
      )
    }
  }

  /**
   * Remove Docker container
   */
  async removeContainer(containerName: string): Promise<void> {
    try {
      await execAsync(`docker rm ${containerName}`)
      this.logger.info({ containerName }, 'Container removed')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      // "removal already in progress" is expected when stop triggers auto-removal
      // "No such container" is expected when container was already removed
      if (
        errorMessage.includes('removal of container') ||
        errorMessage.includes('already in progress') ||
        errorMessage.includes('No such container')
      ) {
        this.logger.debug({ containerName }, 'Container already removed or removal in progress')
      } else {
        this.logger.warn({ error: errorMessage }, 'Failed to remove container')
      }
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
    _repositoryConfig: RepositoryConfig,
    workspacePath: string,
    timeout = 30000,
  ): Promise<{ stdout: string; stderr: string }> {
    // All repositories use workspace path (cloned from remote)
    const containerWorkspacePath = workspacePath
    const dockerCommand = `docker exec -w "${containerWorkspacePath}" ${containerName} /bin/bash -c "${command.replace(/"/g, '\\"')}"`

    const { stdout, stderr } = await execAsync(dockerCommand, { timeout })
    return { stdout, stderr }
  }

  // ===============================
  // Port Management
  // ===============================

  /**
   * Allocate port for preview server
   * Maps host port to container port 3000 (preview server)
   */
  async allocatePreviewPort(): Promise<PortMapping> {
    const previewPort = await getPort()
    return { containerPort: 3000, hostPort: previewPort }
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
