import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { findAvailablePort } from '../../utils/port-allocation.js'
import { DockerEnvironment } from '../docker-environment.js'

const execAsync = promisify(exec)

// Only mock what's necessary for the test environment
vi.mock('../../utils/port-allocation.js')

// Create a mock settings service
const mockSettingsService = {
  getClaudeSettings: () => ({ configDir: '/tmp/.claude' }),
  getDataDir: () => '/tmp/.moatless',
  initialize: () => Promise.resolve(),
} as any

const mockFindAvailablePort = vi.mocked(findAvailablePort)

/**
 * Integration tests for DockerEnvironment with real Docker
 *
 * These tests verify that DockerEnvironment correctly integrates with Docker.
 * They test ONE component (DockerEnvironment) with its external system (Docker).
 *
 * Prerequisites:
 * - Docker must be installed and running
 * - User must have permissions to run Docker commands
 *
 * Note: These are integration tests, not end-to-end tests. They test only
 * DockerEnvironment + Docker, not the full application workflow.
 */
describe('DockerEnvironment - Integration Tests', () => {
  let tempWorkspace: string
  let createdContainers: string[] = []

  beforeAll(async () => {
    // Check if Docker is available
    try {
      await execAsync('docker version')
    } catch (_error) {
      console.log('Docker not available, skipping integration tests')
      return
    }

    // Check if moatless-vibe-universal image exists
    try {
      await execAsync('docker image inspect moatless-vibe-universal:latest')
    } catch (_error) {
      console.warn(
        'moatless-vibe-universal:latest image not found. Please build it first with: docker build -t moatless-vibe-universal:latest -f docker/Dockerfile .',
      )
      throw new Error('Required Docker image not found')
    }
  })

  beforeEach(async () => {
    vi.clearAllMocks()

    // Create a temporary workspace directory
    tempWorkspace = await fs.mkdtemp(path.join(os.tmpdir(), 'docker-env-test-'))

    // Create a test file in the workspace
    await fs.writeFile(path.join(tempWorkspace, 'test.txt'), 'Hello from host')

    // Mock port allocation - use different ports for different test groups
    mockFindAvailablePort.mockResolvedValue(8001)
  })

  afterEach(async () => {
    // Clean up any containers that might have been left behind
    for (const containerId of createdContainers) {
      try {
        await execAsync(`docker rm -f ${containerId}`)
      } catch {
        // Container might already be removed
      }
    }
    createdContainers = []

    // Clean up temp directory
    try {
      await fs.rm(tempWorkspace, { recursive: true, force: true })
    } catch {
      // Directory might already be deleted
    }
  }, 30000)

  // Helper function to create a DockerEnvironment instance
  const createDockerEnvironment = (taskId: string, port = 8001) => {
    mockFindAvailablePort.mockResolvedValue(port)

    const mockRepositoryConfig = {
      id: 'test-repo-integration',
      name: 'Test Repository Integration',
      path: tempWorkspace,
      codeExecution: {
        type: 'docker' as const,
        image: 'moatless-vibe-universal:latest',
        dockerImage: 'moatless-vibe-universal:latest',
        setupCommands: [],
      },
    }

    return new DockerEnvironment(
      taskId,
      mockRepositoryConfig as any,
      tempWorkspace,
      mockSettingsService,
    )
  }

  describe('Container Lifecycle Management', () => {
    let sharedDockerEnvironment: DockerEnvironment

    beforeAll(async () => {
      // Create a temporary workspace directory for this group
      const groupWorkspace = await fs.mkdtemp(path.join(os.tmpdir(), 'docker-env-lifecycle-'))
      await fs.writeFile(path.join(groupWorkspace, 'test.txt'), 'Hello from host')

      // Create and setup a shared container for this test group
      mockFindAvailablePort.mockResolvedValue(8001)
      const mockRepositoryConfig = {
        id: 'test-repo-integration',
        name: 'Test Repository Integration',
        path: groupWorkspace,
        codeExecution: {
          type: 'docker' as const,
          image: 'moatless-vibe-universal:latest',
          dockerImage: 'moatless-vibe-universal:latest',
          setupCommands: [],
        },
      }

      sharedDockerEnvironment = new DockerEnvironment(
        'lifecycle-tests',
        mockRepositoryConfig as any,
        groupWorkspace,
        mockSettingsService,
      )

      // Initially should be not_setup
      const initialStatus = sharedDockerEnvironment.getStatus()
      console.log('Initial status:', initialStatus)
      console.log('Initial error:', sharedDockerEnvironment.getError())
      expect(initialStatus).toBe('not_setup')

      // Setup should create a container
      await sharedDockerEnvironment.setup()

      // Track for cleanup
      createdContainers.push(sharedDockerEnvironment.getContainerName())
    }, 60000)

    afterAll(async () => {
      // Clean up shared container
      try {
        await sharedDockerEnvironment.teardown(true)
      } catch (_error) {
        // Container might not exist
      }
    }, 30000)

    it('should create and setup a real Docker container', async () => {
      // Status should be ready (already setup in beforeAll)
      expect(sharedDockerEnvironment.getStatus()).toBe('ready')

      // Container should exist
      const containerName = sharedDockerEnvironment.getContainerName()
      const { stdout } = await execAsync(
        `docker ps -a --filter "name=${containerName}" --format "{{.Names}}"`,
      )
      expect(stdout.trim()).toBe(containerName)
    }, 10000)

    it('should execute commands in the container', async () => {
      // Execute a simple command
      const result = await sharedDockerEnvironment.runCommand('echo "Hello from Docker"')

      expect(result.exitCode).toBe(0)
      expect(result.output.trim()).toBe('Hello from Docker')
      expect(result.command).toBe('echo "Hello from Docker"')
      expect(result.durationMs).toBeGreaterThan(0)
    }, 10000)

    it('should handle container teardown', async () => {
      // Create a separate container just for this teardown test
      const teardownEnv = createDockerEnvironment('teardown-test', 8002)
      await teardownEnv.setup()
      const containerName = teardownEnv.getContainerName()

      // Teardown should remove the container
      await teardownEnv.teardown()

      // Container should not exist
      const { stdout } = await execAsync(`docker ps -a --filter "name=${containerName}" -q`)
      expect(stdout.trim()).toBe('')
    }, 30000)
  })

  describe('Self-Healing Behavior', () => {
    // These tests need individual containers since they test failure scenarios

    it('should recover from container in error state', async () => {
      // Create individual container for this test
      const healingEnv = createDockerEnvironment('healing-test', 8006)
      await healingEnv.setup()
      const originalContainer = healingEnv.getContainerName()

      // Verify container is running
      let result = await healingEnv.runCommand('echo "Before crash"')
      expect(result.exitCode).toBe(0)

      // Force container into error state by killing it
      await execAsync(`docker kill ${originalContainer}`)

      // Give Docker a moment to update container state
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Next command should trigger recovery
      result = await healingEnv.runCommand('echo "After recovery"')

      // Should succeed after recovery
      expect(result.exitCode).toBe(0)
      expect(result.output.trim()).toBe('After recovery')

      // Should have created a new container (or restarted the same one)
      const { stdout } = await execAsync(
        `docker ps --filter "name=${originalContainer}" --format "{{.Status}}"`,
      )
      expect(stdout).toMatch(/Up/)

      // Clean up
      await healingEnv.teardown(true)
    }, 45000)

    it('should handle missing container gracefully', async () => {
      // Create individual container for this test
      const missingEnv = createDockerEnvironment('missing-test', 8007)
      await missingEnv.setup()
      const containerName = missingEnv.getContainerName()

      // Remove the container manually
      await execAsync(`docker rm -f ${containerName}`)

      // Next operation should recover
      const result = await missingEnv.runCommand('echo "Recovered from missing"')

      expect(result.exitCode).toBe(0)
      expect(result.output.trim()).toBe('Recovered from missing')

      // Clean up
      await missingEnv.teardown(true)
    }, 30000)
  })

  describe('File System Integration', () => {
    let sharedDockerEnvironment: DockerEnvironment

    beforeAll(async () => {
      // Create and setup a shared container for this test group
      sharedDockerEnvironment = createDockerEnvironment('filesystem-tests', 8003)
      await sharedDockerEnvironment.setup()

      // Track for cleanup
      createdContainers.push(sharedDockerEnvironment.getContainerName())
    }, 60000)

    afterAll(async () => {
      // Clean up shared container
      try {
        await sharedDockerEnvironment.teardown(true)
      } catch (_error) {
        // Container might not exist
      }
    }, 30000)

    it('should mount workspace directory correctly', async () => {
      // Check if the test file is accessible in container
      const result = await sharedDockerEnvironment.runCommand('cat test.txt')

      expect(result.exitCode).toBe(0)
      expect(result.output.trim()).toBe('Hello from host')
    }, 10000)

    it('should allow file creation from container visible on host', async () => {
      // Create a file in the container
      const result = await sharedDockerEnvironment.runCommand(
        'echo "Hello from container" > from-container.txt',
      )
      expect(result.exitCode).toBe(0)

      // Verify file exists on host
      const hostPath = path.join(tempWorkspace, 'from-container.txt')
      const content = await fs.readFile(hostPath, 'utf-8')
      expect(content.trim()).toBe('Hello from container')
    }, 10000)
  })

  describe('Setup Commands', () => {
    let sharedDockerEnvironment: DockerEnvironment

    beforeAll(async () => {
      // Create and setup a shared container for this test group
      sharedDockerEnvironment = createDockerEnvironment('setup-tests', 8004)
      await sharedDockerEnvironment.setup()

      // Track for cleanup
      createdContainers.push(sharedDockerEnvironment.getContainerName())
    }, 60000)

    afterAll(async () => {
      // Clean up shared container
      try {
        await sharedDockerEnvironment.teardown(true)
      } catch (_error) {
        // Container might not exist
      }
    }, 30000)

    it('should handle containers without setup commands', async () => {
      // Container should be ready
      expect(sharedDockerEnvironment.getStatus()).toBe('ready')

      // Verify container can execute commands
      const result = await sharedDockerEnvironment.runCommand('echo "No setup commands needed"')
      expect(result.exitCode).toBe(0)
      expect(result.output.trim()).toBe('No setup commands needed')
    }, 10000)
  })

  describe('Error Handling', () => {
    let sharedDockerEnvironment: DockerEnvironment

    beforeAll(async () => {
      // Create and setup a shared container for this test group
      sharedDockerEnvironment = createDockerEnvironment('error-tests', 8005)
      await sharedDockerEnvironment.setup()

      // Track for cleanup
      createdContainers.push(sharedDockerEnvironment.getContainerName())
    }, 60000)

    afterAll(async () => {
      // Clean up shared container
      try {
        await sharedDockerEnvironment.teardown(true)
      } catch (_error) {
        // Container might not exist
      }
    }, 30000)

    it('should handle command failures appropriately', async () => {
      // Execute a failing command
      const result = await sharedDockerEnvironment.runCommand('exit 42')

      expect(result.exitCode).toBe(42)
      expect(result.command).toBe('exit 42')
      expect(result.durationMs).toBeGreaterThan(0)
    }, 10000)

    it('should handle command timeout', async () => {
      // Execute a command that would timeout
      const result = await sharedDockerEnvironment.runCommand('sleep 5', 1000)

      // Command should be killed after timeout
      expect(result.exitCode).not.toBe(0)
      // The command was killed, which may result in empty output or an error message
      expect(result.exitCode).toBeTruthy()
    }, 10000)
  })

  describe('Container Status Tracking', () => {
    // These tests need individual containers to test specific status scenarios

    it('should correctly report container status', async () => {
      // Create individual container for status testing
      const statusEnv = createDockerEnvironment('status-test', 8008)

      // Not setup initially
      expect(statusEnv.getStatus()).toBe('not_setup')
      expect(statusEnv.getError()).toBeUndefined()

      // Setup the container
      await statusEnv.setup()

      // Should be ready after setup
      expect(statusEnv.getStatus()).toBe('ready')
      expect(statusEnv.getError()).toBeUndefined()

      // Clean up
      await statusEnv.teardown(true)
    }, 30000)

    it('should track error state appropriately', async () => {
      // Create environment with invalid image to force an error
      mockFindAvailablePort.mockResolvedValue(8009)

      const errorConfig = {
        id: 'error-repo',
        name: 'Error Repository',
        path: tempWorkspace,
        codeExecution: {
          type: 'docker' as const,
          image: 'nonexistent-image:does-not-exist',
          dockerImage: 'nonexistent-image:does-not-exist',
        },
      }

      const errorEnv = new DockerEnvironment(
        'error-task',
        errorConfig as any,
        tempWorkspace,
        mockSettingsService,
      )

      // Setup should fail
      await expect(errorEnv.setup()).rejects.toThrow()

      // After a failed setup, the environment should report error
      // The error is stored in the _error field
      expect(errorEnv.getError()).toBeDefined()
      expect(errorEnv.getError()).toContain('nonexistent-image')
    }, 30000)
  })
})
