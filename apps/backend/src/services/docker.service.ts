import { exec, execSync } from 'node:child_process'
import { chmodSync, chownSync, existsSync, mkdirSync } from 'node:fs'
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
import { PluginService } from '../features/plugins/services/plugin.service'
import { buildServersConfig } from '../features/workspace/utils/server-ports'
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

  constructor(
    @inject(SettingsService) private settingsService: SettingsService,
    @inject(PluginService) private pluginService: PluginService,
  ) {}

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
   * Returns unified EnvironmentStatus values
   *
   * Docker daemon states map to unified status:
   * - (not found) → 'not_created'
   * - 'created'   → 'initializing'
   * - 'running'   → 'ready'
   * - 'exited'    → 'stopped' (container was running but stopped - not an error)
   * - 'dead'      → 'error' (container crashed)
   * - 'removing'  → 'removing'
   */
  getContainerStatus(
    containerName: string,
  ): 'not_created' | 'initializing' | 'ready' | 'stopped' | 'removing' | 'error' {
    try {
      const containerStatus = execSync(
        `docker inspect --format='{{.State.Status}}' "${containerName}"`,
        { encoding: 'utf8', timeout: 2000, stdio: 'pipe' },
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
        return 'removing'
      }

      // 'exited' means the container stopped normally or was stopped externally
      // This is NOT an error - it allows re-provisioning
      if (containerStatus === 'exited') {
        return 'stopped'
      }

      // 'dead' means the container crashed or failed
      if (containerStatus === 'dead') {
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
    this.logger.info({ containerId }, 'Removing existing container')

    // First try to stop the container if it's running
    try {
      await execAsync(`docker stop ${containerId}`)
      this.logger.debug({ containerId }, 'Container stopped')
    } catch (_stopError) {
      // Container might already be stopped, continue with removal
      this.logger.debug({ containerId }, 'Container stop failed (may already be stopped)')
    }

    // Remove the container
    try {
      await execAsync(`docker rm ${containerId}`)
      this.logger.info({ containerId }, 'Container removed successfully')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      if (
        errorMessage.includes('removal of container') ||
        errorMessage.includes('already in progress') ||
        errorMessage.includes('No such container')
      ) {
        this.logger.debug({ containerId }, 'Container already removed or removal in progress')
      } else {
        this.logger.error(
          { err: error as Error, containerId },
          'Failed to remove existing container',
        )
        throw error
      }
    }
  }

  /**
   * Create Docker container with full configuration
   *
   * @param containerName - Name of the container
   * @param taskId - Task ID for labeling and environment
   * @param repositoryConfig - Repository configuration
   * @param workspacePath - Path to workspace inside container
   * @param portMappings - Array of port mappings (container → host) for all servers and sandbox API
   * @param image - Docker image to use
   * @param githubToken - Optional GitHub token for git operations
   * @param taskAuthToken - Optional JWT token for sandbox-to-backend communication
   */
  async createContainer(
    containerName: string,
    taskId: string,
    repositoryConfig: RepositoryConfig,
    workspacePath: string,
    portMappings: PortMapping[],
    image: string,
    githubToken?: string | null,
    taskAuthToken?: string,
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
    for (const mapping of portMappings) {
      createCommand = this.addPortMapping(createCommand, mapping)
    }
    createCommand = this.addWorkerEnvironment(
      createCommand,
      taskId,
      containerWorkspacePath,
      taskAuthToken,
    )
    createCommand = this.addClaudeEnvironment(createCommand)
    createCommand = await this.addRepositoryEnvironment(
      createCommand,
      repositoryConfig,
      githubToken,
    )

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
    command += ` --label "moatless.service=sandbox"`
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
    // DATA_DIR is the HOST path used for Docker volume mounts (sibling containers)
    // CONTAINER_DATA_DIR is where the data is mounted inside this container (for file ops)
    // In local dev they're typically the same; in docker-compose CONTAINER_DATA_DIR=/data
    const hostDataDir = process.env['DATA_DIR'] || './data'
    const containerDataDir = process.env['CONTAINER_DATA_DIR'] || hostDataDir

    // Create per-task directory for isolated storage (similar to Kubernetes PVCs)
    // Use host path for Docker volume mounts, container path for file operations
    const taskDirHost = path.join(hostDataDir, 'tasks', taskId)
    const taskDirContainer = path.join(containerDataDir, 'tasks', taskId)

    // Ensure task directory and subdirectories exist with proper permissions
    // The sandbox container runs as moatless user (UID 1000), so all mounted
    // directories must be owned by UID 1000 and writable
    const SANDBOX_UID = 1000
    const SANDBOX_GID = 1000
    const subdirs = ['repo', 'data', '.claude', '.claude/debug']
    for (const subdir of ['', ...subdirs]) {
      const dir = subdir ? path.join(taskDirContainer, subdir) : taskDirContainer
      if (!existsSync(dir)) {
        mkdirSync(dir, { recursive: true, mode: 0o777 })
      }
      chmodSync(dir, 0o777)
      // Set ownership to sandbox user (UID 1000) - required for Docker volume mounts
      // This may fail on non-root systems but that's okay since local dev usually works
      try {
        chownSync(dir, SANDBOX_UID, SANDBOX_GID)
      } catch {
        // Ignore chown errors on systems where we don't have permission
      }
    }

    // Use HOST path for Docker volume mount (sibling container needs host path)
    const taskDir = taskDirHost

    let result = command
    // Note: We don't mount a workspace volume because the container clones
    // the repository internally using GIT_CLONE_URL environment variable.
    // The workspace lives entirely inside the container.
    // Mount task-specific directory to /opt/moatless for isolated storage per task
    // This provides: /opt/moatless/repo, /opt/moatless/data, /opt/moatless/.claude
    result += ` -v "${taskDir}:/opt/moatless"`

    // Mount plugins directory as read-only for Claude Code plugins
    // Use container path for existsSync, host path for Docker volume mount
    // Mount outside .claude to avoid root-owned intermediate dirs on K8s
    const pluginsDirContainer = path.join(containerDataDir, 'plugins')
    const pluginsDirHost = path.join(hostDataDir, 'plugins')
    const sandboxPluginsDir = process.env['SANDBOX_PLUGINS_DIR'] || '/mnt/plugins'
    if (existsSync(pluginsDirContainer)) {
      result += ` -v "${pluginsDirHost}:${sandboxPluginsDir}:ro"`
      this.logger.debug({ pluginsDirHost, sandboxPluginsDir }, 'Plugins volume mounted')
    }

    // Mount uploads directory as read-only for file attachments
    // This allows sandbox containers to access uploaded files without base64 transfer
    // Use UPLOAD_DIR if set, otherwise default to {DATA_DIR}/uploads
    const uploadsDirHost = process.env['UPLOAD_DIR'] || path.join(hostDataDir, 'uploads')
    const uploadsDirContainer = process.env['UPLOAD_DIR'] || path.join(containerDataDir, 'uploads')
    const sandboxUploadMountPath = process.env['SANDBOX_UPLOAD_MOUNT_PATH'] || '/mnt/uploads'
    if (existsSync(uploadsDirContainer)) {
      result += ` -v "${uploadsDirHost}:${sandboxUploadMountPath}:ro"`
      this.logger.debug({ uploadsDirHost, sandboxUploadMountPath }, 'Uploads volume mounted')
    }

    return result
  }

  private addClaudeConfigMount(command: string): string {
    // Claude config is stored in /opt/moatless/.claude which is already mounted via addVolumeMappings()
    // No separate mount needed - CLAUDE_CONFIG_DIR is set in addClaudeEnvironment()
    return command
  }

  private addPortMapping(command: string, workerPortMapping: PortMapping): string {
    return `${command} -p ${workerPortMapping.hostPort}:${workerPortMapping.containerPort}`
  }

  private addWorkerEnvironment(
    command: string,
    taskId: string,
    containerWorkspacePath: string,
    taskAuthToken?: string,
  ): string {
    const port = process.env['PORT'] || '8000'
    const backendApiUrl =
      process.env['BACKEND_API_URL'] ||
      process.env['API_URL'] ||
      `http://host.docker.internal:${port}`

    let result = command
    result += ` -e "BACKEND_API_URL=${backendApiUrl}"`
    result += ` -e "TASK_ID=${taskId}"`
    result += ` -e "WORKSPACE_DIR=${containerWorkspacePath}"`
    result += ` -e "DATA_DIR=${SANDBOX_DATA_DIR}"`
    result += ` -e "LOG_LEVEL=info"`
    // Set HOME to the moatless user's home directory, not the workspace
    // The workspace must be empty for git clone to succeed
    result += ` -e "HOME=${SANDBOX_HOME}"`

    // Redis configuration for event publishing
    // Replace localhost/127.0.0.1 with host.docker.internal so the container can reach the host's Redis
    const redisUrl = process.env['REDIS_URL'] || 'redis://localhost:6379'
    const containerRedisUrl = redisUrl
      .replace('://localhost', '://host.docker.internal')
      .replace('://127.0.0.1', '://host.docker.internal')
    result += ` -e "REDIS_URL=${containerRedisUrl}"`

    // Task-scoped JWT token for sandbox-to-backend MCP communication
    if (taskAuthToken) {
      result += ` -e "TASK_AUTH_TOKEN=${taskAuthToken}"`
    }

    return result
  }

  private addClaudeEnvironment(command: string): string {
    const claudeSettings = this.settingsService.getClaudeSettings()

    let result = command

    // Set CLAUDE_CONFIG_DIR to /opt/moatless/.claude for persistence across container restarts
    result += ` -e "CLAUDE_CONFIG_DIR=${SANDBOX_CLAUDE_CONFIG_DIR}"`

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

    return result
  }

  private async addRepositoryEnvironment(
    command: string,
    repositoryConfig: RepositoryConfig,
    githubToken?: string | null,
  ): Promise<string> {
    let result = command

    // Add GitHub token if available (for git push operations)
    if (githubToken) {
      result += ` -e "GITHUB_TOKEN=${githubToken}"`
    }

    // Pass git clone URL so the sandbox can clone the repository internally
    if (repositoryConfig.remote?.url) {
      result += ` -e "GIT_CLONE_URL=${repositoryConfig.remote.url}"`
    }

    // Pass user-defined environment variables individually
    const envVars = repositoryConfig.codeExecution.envVars || {}
    for (const [key, value] of Object.entries(envVars)) {
      result += ` -e "${key}=${value}"`
    }

    // Resolve plugin IDs to relative paths using PluginService
    const pluginPaths: string[] = []
    if (repositoryConfig.plugins && repositoryConfig.plugins.length > 0) {
      for (const pluginId of repositoryConfig.plugins) {
        const relativePath = await this.pluginService.getPluginRelativePath(pluginId)
        if (relativePath) {
          pluginPaths.push(relativePath)
        } else {
          this.logger.warn({ pluginId }, 'Could not resolve plugin path, skipping')
        }
      }
    }

    // Build servers config with dynamically allocated ports
    const serversWithPorts = buildServersConfig(repositoryConfig)

    // Pass full repo config as single JSON env var (same format as .moatless/config.json)
    // Includes: systemPrompt, codeExecution (setupCommands, dockerImage), previewConfig, servers, agentConfig
    // This enables sandbox to auto-start servers immediately at startup
    // and read agent configuration (plugins, timeout, model) from config instead of per-request
    const repoConfig = {
      systemPrompt: repositoryConfig.systemPrompt,
      codeExecution: {
        setupCommands: repositoryConfig.codeExecution.setupCommands,
        dockerImage: repositoryConfig.codeExecution.dockerImage,
        // envVars already passed separately above
      },
      previewConfig: repositoryConfig.previewConfig,
      servers: serversWithPorts,
      // Agent configuration: static settings that apply to all requests
      agentConfig: {
        plugins: repositoryConfig.plugins,
        pluginPaths: pluginPaths.length > 0 ? pluginPaths : undefined,
        // timeout and model can be added here when repository supports these settings
      },
    }

    // Always pass REPO_CONFIG — servers array is always present (at least built-in "files")
    const repoConfigJson = JSON.stringify(repoConfig)
    // Escape single quotes for shell safety
    result += ` -e 'REPO_CONFIG=${repoConfigJson.replace(/'/g, "'\\''")}'`

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
    // Check if container exists first
    const status = this.getContainerStatus(containerName)
    if (status === 'not_created') {
      this.logger.debug({ containerName }, 'Container does not exist, skipping stop')
      return
    }

    try {
      await execAsync(`docker stop ${containerName}`)
      this.logger.info({ containerName }, 'Container stopped')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      // "No such container" can still happen in race conditions
      if (errorMessage.includes('No such container')) {
        this.logger.debug({ containerName }, 'Container already removed')
      } else {
        this.logger.warn({ error: errorMessage }, 'Failed to stop container')
      }
    }
  }

  /**
   * Get container logs for debugging
   * Returns the last N lines of container stdout/stderr
   */
  async getContainerLogs(containerName: string, tail = 50): Promise<string | null> {
    try {
      const { stdout } = await execAsync(`docker logs --tail ${tail} "${containerName}" 2>&1`, {
        timeout: 5000,
      })
      return stdout
    } catch (error) {
      this.logger.debug({ containerName, error }, 'Failed to get container logs')
      return null
    }
  }

  /**
   * Remove Docker container
   */
  async removeContainer(containerName: string): Promise<void> {
    // Check if container exists first
    const status = this.getContainerStatus(containerName)
    if (status === 'not_created') {
      this.logger.debug({ containerName }, 'Container does not exist, skipping remove')
      return
    }

    try {
      await execAsync(`docker rm ${containerName}`)
      this.logger.info({ containerName }, 'Container removed')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      // "removal already in progress" is expected when stop triggers auto-removal
      // "No such container" can still happen in race conditions
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
   * Allocate a host port for a container port
   */
  async allocatePort(containerPort: number): Promise<PortMapping> {
    const hostPort = await getPort()
    return { containerPort, hostPort }
  }

  /**
   * Allocate port for sandbox API
   * Maps host port to container port 8001 (sandbox HTTP server)
   * Used for health checks and direct API access from host
   */
  allocateSandboxApiPort(): Promise<PortMapping> {
    return this.allocatePort(8001)
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
