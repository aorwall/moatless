import { inject, injectable } from 'inversify'
import { DockerService } from '../../../services/docker.service'
import { logger } from '../../../utils/logger'
import { EventService } from '../../events/services/event.service'
import {
  provisionerCleanupTotal,
  provisionerDriverCreatedTotal,
  provisionerGetStatusTotal,
  provisionerHealthCheckDurationSeconds,
  provisionerHealthCheckTotal,
  workspaceProvisionDurationSeconds,
  workspaceProvisionTotal,
} from '../../metrics'
import type { BaseEnvironmentDriver } from '../drivers/base-environment-driver'
import { RemoteEnvironmentDriver } from '../drivers/remote-environment-driver'
import type { WorkspaceProvisionPlan } from '../types/provision-plan.types'
import type {
  WorkspaceProvisioner,
  WorkspaceStatusResult,
} from '../types/workspace-provisioner.types'
import {
  CONTAINER_WORKSPACE_PATH,
  type DockerExecutionTarget,
  type ExecutionTarget,
  isDockerExecutionTarget,
} from '../types/workspace-runtime.types'

/**
 * DockerWorkspaceProvisioner - Handles Docker workspace lifecycle
 *
 * Implements WorkspaceProvisioner for Docker container-based workspaces.
 * Manages container creation, health checking, and cleanup.
 *
 * Responsibilities:
 * - Provision workspaces by creating Docker containers
 * - Check container health via HTTP endpoint
 * - Validate container exists and is running
 * - Query agent status from worker container
 * - Clean up containers
 * - Create RemoteEnvironmentDriver instances
 */
@injectable()
export class DockerWorkspaceProvisioner implements WorkspaceProvisioner {
  private readonly provisionerLogger = logger.child({ service: 'DockerWorkspaceProvisioner' })

  constructor(
    @inject(DockerService) private readonly dockerService: DockerService,
    @inject(EventService) private readonly eventService: EventService,
  ) {}

  /**
   * Provision a Docker workspace by creating a container
   *
   * Steps:
   * 1. Get repository configuration and git clone URL
   * 2. Resolve authentication (GitHub token, Claude token, worker auth)
   * 3. Create container with environment configuration
   * 4. Start container and wait for health
   *
   * @returns DockerExecutionTarget with container info
   */
  async provision(taskId: string, plan: WorkspaceProvisionPlan): Promise<ExecutionTarget> {
    const startTime = Date.now()
    const { repoConfig, gitCloneUrl, gitBranch, gitProviderToken } = plan
    this.provisionerLogger.info(
      { taskId, pushEnabled: plan.pushEnabled },
      'DockerWorkspaceProvisioner: provision()',
    )

    // 1. Generate container name and check for existing container
    const containerName = this.dockerService.generateContainerName(repoConfig.name, taskId)

    const existingTarget = await this.checkExistingContainer(
      taskId,
      containerName,
      gitCloneUrl,
      gitBranch,
    )
    if (existingTarget) {
      this.provisionerLogger.info(
        { taskId, containerName },
        'DockerWorkspaceProvisioner: Reusing existing container',
      )
      return existingTarget
    }

    this.provisionerLogger.info(
      { taskId, containerName, gitCloneUrl },
      'DockerWorkspaceProvisioner: Creating Docker container',
    )

    // 2. Allocate ports (preview for browser access, sandbox API for health checks and communication)
    const previewPortMapping = await this.dockerService.allocatePreviewPort()
    const sandboxApiPortMapping = await this.dockerService.allocateSandboxApiPort()

    // 3. Get Docker settings from environment
    const workerImage =
      repoConfig.codeExecution?.dockerImage ||
      process.env['DOCKER_SANDBOX_IMAGE'] ||
      'ghcr.io/aorwall/moatless-vibe-sandbox-js:latest'

    // Standard workspace path inside container (matches sandbox WORKSPACE_DIR)
    const workspacePath = CONTAINER_WORKSPACE_PATH

    try {
      // 4. Create container (preview port + sandbox API port mapped to host)
      await this.dockerService.createContainer(
        containerName,
        taskId,
        repoConfig,
        workspacePath,
        previewPortMapping,
        sandboxApiPortMapping,
        workerImage,
        gitProviderToken,
      )

      // 5. Start container
      await this.dockerService.startContainer(containerName)

      // 6. Wait for container to be healthy and workspace provisioned (git clone complete)
      // Use localhost with mapped host port for health checks (works on macOS Docker Desktop)
      await this.waitForWorkspaceReady(containerName, sandboxApiPortMapping.hostPort, 60000)

      // 7. Create execution target
      const target: DockerExecutionTarget = {
        mode: 'docker',
        containerName,
        status: 'ready',
        previewHostPort: previewPortMapping.hostPort,
        sandboxApiHostPort: sandboxApiPortMapping.hostPort,
        repositoryCloneUrl: gitCloneUrl,
      }

      if (gitBranch) {
        target.repositoryBranch = gitBranch
      }

      // Track metrics
      const durationSeconds = (Date.now() - startTime) / 1000
      workspaceProvisionTotal.labels({ mode: 'docker', status: 'success' }).inc()
      workspaceProvisionDurationSeconds.labels({ mode: 'docker' }).observe(durationSeconds)

      this.provisionerLogger.info(
        { taskId, containerName, previewHostPort: previewPortMapping.hostPort },
        'DockerWorkspaceProvisioner: Workspace provisioned',
      )

      return target
    } catch (error) {
      // Track error metrics
      workspaceProvisionTotal.labels({ mode: 'docker', status: 'error' }).inc()

      const errorMessage = error instanceof Error ? error.message : String(error)
      this.provisionerLogger.error(
        { taskId, containerName, error: errorMessage },
        'DockerWorkspaceProvisioner: Container provisioning failed',
      )

      // Cleanup failed container
      try {
        await this.dockerService.stopContainer(containerName)
        await this.dockerService.removeContainer(containerName)
      } catch {
        // Ignore cleanup errors
      }

      throw error
    }
  }

  /**
   * Wait for container to be healthy AND workspace to be provisioned
   * This ensures git clone and any setup commands have completed
   *
   * Uses localhost with mapped host port for health checks (accessible from host on macOS)
   */
  private async waitForWorkspaceReady(
    containerName: string,
    sandboxApiHostPort: number,
    timeoutMs: number,
  ): Promise<void> {
    const startTime = Date.now()
    const sandboxUrl = `http://localhost:${sandboxApiHostPort}`
    let pollCount = 0

    while (Date.now() - startTime < timeoutMs) {
      pollCount++

      // Check if container is still running (catches early crashes)
      await this.assertContainerRunning(containerName, pollCount)

      try {
        const isReady = await this.checkWorkspaceHealth(sandboxUrl, containerName, pollCount)
        if (isReady) {
          return
        }
      } catch (error) {
        this.handleHealthCheckError(error, containerName, pollCount)
      }

      await new Promise((resolve) => setTimeout(resolve, 1000))
    }

    await this.handleProvisioningTimeout(containerName, pollCount, timeoutMs)
  }

  /**
   * Assert that the container is still running, throw if it has exited
   */
  private async assertContainerRunning(containerName: string, pollCount: number): Promise<void> {
    const containerStatus = this.dockerService.getContainerStatus(containerName)
    if (containerStatus === 'ready' || containerStatus === 'initializing') {
      return
    }

    this.provisionerLogger.error(
      { containerName, containerStatus, pollCount },
      'DockerWorkspaceProvisioner: Container exited unexpectedly during provisioning',
    )
    await this.logContainerLogs(containerName, 'Container logs before exit')
    throw new Error(`Container exited with status: ${containerStatus}`)
  }

  /**
   * Check container health and workspace status
   * Returns true if workspace is ready, false if still pending
   */
  private async checkWorkspaceHealth(
    sandboxUrl: string,
    containerName: string,
    pollCount: number,
  ): Promise<boolean> {
    // 1. Check health endpoint
    const healthResponse = await fetch(`${sandboxUrl}/health`, {
      signal: AbortSignal.timeout(5000),
    })

    if (!healthResponse.ok) {
      this.provisionerLogger.info(
        { containerName, statusCode: healthResponse.status, pollCount },
        'DockerWorkspaceProvisioner: Health check returned non-OK status',
      )
      return false
    }

    // 2. Check workspace provisioning status (git clone completion)
    const statusResponse = await fetch(`${sandboxUrl}/workspace/status`, {
      signal: AbortSignal.timeout(5000),
    })

    if (!statusResponse.ok) {
      return false
    }

    const statusData = (await statusResponse.json()) as { status: string; error?: string }

    if (statusData.status === 'ready') {
      this.provisionerLogger.info(
        { containerName, pollCount },
        'DockerWorkspaceProvisioner: Container and workspace ready',
      )
      return true
    }

    if (statusData.status === 'failed') {
      throw new Error(`Workspace provisioning failed: ${statusData.error || 'unknown error'}`)
    }

    // Still pending (e.g., cloning), continue polling
    this.provisionerLogger.info(
      { containerName, status: statusData.status, pollCount },
      'DockerWorkspaceProvisioner: Workspace provisioning in progress',
    )
    return false
  }

  /**
   * Handle errors during health check, re-throw critical errors
   */
  private handleHealthCheckError(error: unknown, containerName: string, pollCount: number): void {
    // Re-throw critical errors
    if (
      error instanceof Error &&
      (error.message.includes('provisioning failed') || error.message.includes('Container exited'))
    ) {
      throw error
    }
    // Container not ready yet, log and continue polling
    const errorMessage = error instanceof Error ? error.message : String(error)
    this.provisionerLogger.info(
      { containerName, error: errorMessage, pollCount },
      'DockerWorkspaceProvisioner: Health check failed, container not ready yet',
    )
  }

  /**
   * Handle provisioning timeout - log and throw error
   */
  private async handleProvisioningTimeout(
    containerName: string,
    pollCount: number,
    timeoutMs: number,
  ): Promise<never> {
    this.provisionerLogger.error(
      { containerName, pollCount, timeoutMs },
      'DockerWorkspaceProvisioner: Workspace provisioning timed out',
    )
    await this.logContainerLogs(containerName, 'Container logs at timeout')
    throw new Error(`Workspace provisioning timed out after ${timeoutMs}ms`)
  }

  /**
   * Log container logs for debugging
   */
  private async logContainerLogs(containerName: string, message: string): Promise<void> {
    const logs = await this.dockerService.getContainerLogs(containerName)
    if (logs) {
      this.provisionerLogger.error(
        { containerName, logs: logs.slice(-2000) }, // Limit log size
        `DockerWorkspaceProvisioner: ${message}`,
      )
    }
  }

  /**
   * Check if container is healthy via HTTP health endpoint
   * Uses localhost with mapped sandbox API host port
   */
  async isHealthy(_taskId: string, target: ExecutionTarget): Promise<boolean> {
    const startTime = Date.now()

    if (!isDockerExecutionTarget(target)) {
      this.provisionerLogger.warn(
        'DockerWorkspaceProvisioner: isHealthy called with non-docker target',
      )
      provisionerHealthCheckTotal.labels({ mode: 'docker', result: 'error' }).inc()
      return false
    }

    if (!target.sandboxApiHostPort) {
      provisionerHealthCheckTotal.labels({ mode: 'docker', result: 'error' }).inc()
      return false
    }

    try {
      const sandboxUrl = `http://localhost:${target.sandboxApiHostPort}`
      const response = await fetch(`${sandboxUrl}/health`, {
        signal: AbortSignal.timeout(5000),
      })

      const isHealthy = response.ok

      // Track metrics
      const durationSeconds = (Date.now() - startTime) / 1000
      provisionerHealthCheckDurationSeconds.labels({ mode: 'docker' }).observe(durationSeconds)
      provisionerHealthCheckTotal
        .labels({ mode: 'docker', result: isHealthy ? 'healthy' : 'unhealthy' })
        .inc()

      if (!isHealthy) {
        this.provisionerLogger.debug(
          { containerName: target.containerName },
          'DockerWorkspaceProvisioner: Container health check failed',
        )
      }

      return isHealthy
    } catch (error) {
      const durationSeconds = (Date.now() - startTime) / 1000
      provisionerHealthCheckDurationSeconds.labels({ mode: 'docker' }).observe(durationSeconds)
      provisionerHealthCheckTotal.labels({ mode: 'docker', result: 'unhealthy' }).inc()

      this.provisionerLogger.debug(
        { containerName: target.containerName, err: error },
        'DockerWorkspaceProvisioner: Container unreachable',
      )
      return false
    }
  }

  /**
   * Get container status for logging and decision making
   */
  getStatus(_taskId: string, target: ExecutionTarget): WorkspaceStatusResult {
    if (!isDockerExecutionTarget(target)) {
      provisionerGetStatusTotal.labels({ mode: 'docker', status: 'error' }).inc()
      return { status: 'error', lastError: 'Non-docker target' }
    }

    const status = this.dockerService.getContainerStatus(target.containerName)

    provisionerGetStatusTotal.labels({ mode: 'docker', status }).inc()

    return {
      status,
      containerName: target.containerName,
      lastError: target.lastError,
    }
  }

  /**
   * Check if an agent is currently running by querying worker container's /agent-status endpoint
   * Uses localhost with mapped sandbox API host port
   */
  async isAgentRunning(taskId: string, target: ExecutionTarget): Promise<boolean> {
    if (!isDockerExecutionTarget(target)) {
      return false
    }

    if (!target.sandboxApiHostPort) {
      return false
    }

    try {
      const sandboxUrl = `http://localhost:${target.sandboxApiHostPort}`
      const response = await fetch(`${sandboxUrl}/agent-status`, {
        signal: AbortSignal.timeout(5000),
      })

      if (!response.ok) {
        return false
      }

      const data = (await response.json()) as { state: string }
      return data.state === 'running'
    } catch (error) {
      this.provisionerLogger.debug(
        { taskId, containerName: target.containerName, err: error },
        'DockerWorkspaceProvisioner: Worker unreachable',
      )
      return false
    }
  }

  /**
   * Clean up by stopping and removing the container
   */
  async cleanup(
    taskId: string,
    target: ExecutionTarget,
    _plan?: WorkspaceProvisionPlan,
  ): Promise<void> {
    if (!isDockerExecutionTarget(target)) {
      this.provisionerLogger.warn(
        'DockerWorkspaceProvisioner: cleanup called with non-docker target',
      )
      return
    }

    this.provisionerLogger.info(
      { taskId, containerName: target.containerName },
      'DockerWorkspaceProvisioner: Stopping and removing container',
    )

    try {
      await this.dockerService.stopContainer(target.containerName)
      await this.dockerService.removeContainer(target.containerName)
      provisionerCleanupTotal.labels({ mode: 'docker', status: 'success' }).inc()
      this.provisionerLogger.info({ taskId }, 'DockerWorkspaceProvisioner: Container removed')
    } catch (error) {
      provisionerCleanupTotal.labels({ mode: 'docker', status: 'error' }).inc()
      this.provisionerLogger.warn(
        { taskId, error: (error as Error).message },
        'DockerWorkspaceProvisioner: Failed to remove container (may already be removed)',
      )
    }
  }

  /**
   * Clear error state by removing the failed container
   */
  async clearError(taskId: string, target: ExecutionTarget): Promise<void> {
    await this.cleanup(taskId, target)
  }

  /**
   * Create a RemoteEnvironmentDriver for the workspace
   *
   * Uses localhost with mapped sandbox API host port for communication
   * Provides external preview URL via previewHostPort for browser access
   */
  createDriver(taskId: string, target: ExecutionTarget): BaseEnvironmentDriver {
    if (!isDockerExecutionTarget(target)) {
      throw new Error('DockerWorkspaceProvisioner: Cannot create driver for non-docker target')
    }

    provisionerDriverCreatedTotal.labels({ mode: 'docker' }).inc()

    return new RemoteEnvironmentDriver({
      taskId,
      // Use localhost with mapped host port for sandbox API (works on macOS Docker Desktop)
      getWorkerUrl: async () => `http://localhost:${target.sandboxApiHostPort}`,
      eventService: this.eventService,
      // Provide external preview URL for browser access (direct host port)
      getExternalPreviewUrl: () =>
        target.previewHostPort ? `http://localhost:${target.previewHostPort}` : undefined,
    })
  }

  /**
   * Check if an existing container can be reused
   * Returns the execution target if container is healthy, null otherwise
   */
  private async checkExistingContainer(
    taskId: string,
    containerName: string,
    _repositoryCloneUrl: string,
    _repositoryBranch?: string,
  ): Promise<ExecutionTarget | null> {
    const existingContainer = await this.dockerService.getExistingContainer(containerName)
    if (!existingContainer) {
      return null
    }

    // Get container info to find the host port
    const status = this.dockerService.getContainerStatus(containerName)

    if (status === 'ready') {
      // Container is running - try to get host port from docker inspect
      // For now, we'll need to remove and recreate since we don't store the port mapping
      this.provisionerLogger.info(
        { taskId, containerName },
        'DockerWorkspaceProvisioner: Found running container, but port mapping unknown - recreating',
      )
      await this.dockerService.stopContainer(containerName)
      await this.dockerService.removeContainer(containerName)
      return null
    }

    // Container exists but not ready - remove it
    if (existingContainer) {
      this.provisionerLogger.info(
        { taskId, containerName, status: existingContainer.status },
        'DockerWorkspaceProvisioner: Removing existing non-running container',
      )
      await this.dockerService.removeExistingContainer(existingContainer.id)
    }

    return null
  }
}
