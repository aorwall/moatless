import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { container } from 'tsyringe'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'

// Helper function to create complete RepositoryConfig for tests
function createMockRepository(overrides: Partial<RepositoryConfig>): RepositoryConfig {
  return {
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    ...overrides,
  } as RepositoryConfig
}

import { type DockerService, DockerService as DockerServiceClass } from '../docker.service.js'
import { SettingsService } from '../settings.service.js'

/**
 * Integration tests for DockerService with real Docker
 *
 * These tests verify that DockerService correctly integrates with Docker daemon.
 * They test the DockerService in isolation with real Docker operations.
 *
 * Prerequisites:
 * - Docker must be installed and running
 * - User must have permissions to run Docker commands
 * - CLAUDE_CODE_OAUTH_TOKEN environment variable should be set
 */
describe('DockerService - Integration Tests', () => {
  let dockerService: DockerService
  let tempWorkspace: string
  let createdContainers: string[] = []

  beforeAll(async () => {
    // Create services following DI pattern
    const _settingsService = container.resolve(SettingsService)
    dockerService = container.resolve(DockerServiceClass)

    // Check if Docker is available and skip tests if not
    const dockerCheck = await dockerService.checkDockerAvailable()
    if (!dockerCheck.available) {
      console.log(`Skipping Docker integration tests: ${dockerCheck.message}`)
      return
    }

    // Create temporary workspace directory
    tempWorkspace = await fs.mkdtemp(path.join(os.tmpdir(), 'docker-service-test-'))

    // Create a basic test file in workspace
    await fs.writeFile(path.join(tempWorkspace, 'test-file.txt'), 'Hello Docker!')
  })

  afterEach(async () => {
    // Clean up any containers created during tests
    for (const containerName of createdContainers) {
      try {
        await dockerService.stopContainer(containerName)
        await dockerService.removeContainer(containerName)
      } catch {
        // Container might not exist or already removed
      }
    }
    createdContainers = []
  })

  describe('Docker Environment Detection', () => {
    it('should detect if Docker is running', async () => {
      const isRunning = await dockerService.isDockerRunning()
      expect(isRunning).toBe(true)
    })

    it('should get Docker version', async () => {
      const version = await dockerService.getDockerVersion()
      expect(version).toBeTruthy()
      expect(version).toMatch(/Docker version/)
    })

    it('should check Docker availability with OAuth token', async () => {
      const check = await dockerService.checkDockerAvailable()
      expect(check.dockerRunning).toBe(true)
      expect(check.available).toBe(true)
    })
  })

  describe('Container Name Generation', () => {
    it('should generate consistent container names', () => {
      const name1 = dockerService.generateContainerName('test-repo', 'task-123')
      const name2 = dockerService.generateContainerName('test-repo', 'task-123')

      expect(name1).toBe(name2)
      expect(name1).toMatch(/moatless-test-repo-task-123/)
    })

    it('should generate different names for different tasks', () => {
      const name1 = dockerService.generateContainerName('test-repo', 'task-123')
      const name2 = dockerService.generateContainerName('test-repo', 'task-456')

      expect(name1).not.toBe(name2)
    })
  })

  describe('Container Status Management', () => {
    it('should return not_created for non-existent container', () => {
      const status = dockerService.getContainerStatus('non-existent-container')
      expect(status).toBe('not_created')
    })

    it('should detect existing containers', async () => {
      const containerName = dockerService.generateContainerName('test-detect', 'task-detect')
      createdContainers.push(containerName)

      // Create a simple container that exits immediately
      const mockRepo = createMockRepository({
        id: 'test-detect',
        name: 'test-detect',
        path: '/workspace',
        codeExecution: { type: 'docker', envVars: {}, setupCommands: [] },
        useGitWorktrees: false,
      })

      const portMapping = await dockerService.allocateWorkerPort()

      // Create container
      await dockerService.createContainer(
        containerName,
        'task-detect',
        mockRepo,
        tempWorkspace,
        portMapping,
        'alpine:latest',
      )

      // Check status
      const status = dockerService.getContainerStatus(containerName)
      expect(['initializing', 'ready', 'error']).toContain(status)
    })
  })

  describe('Port Allocation', () => {
    it('should allocate available worker ports', async () => {
      const port1 = await dockerService.allocateWorkerPort()
      const port2 = await dockerService.allocateWorkerPort()

      expect(port1.containerPort).toBe(8001)
      expect(port2.containerPort).toBe(8001)
      expect(port1.hostPort).not.toBe(port2.hostPort)
      expect(port1.hostPort).toBeGreaterThan(8000)
      expect(port2.hostPort).toBeGreaterThan(8000)
    })
  })

  describe('Container Lifecycle Management', () => {
    it('should create and manage container lifecycle', async () => {
      const containerName = dockerService.generateContainerName('test-lifecycle', 'task-lifecycle')
      createdContainers.push(containerName)

      const mockRepo = createMockRepository({
        id: 'test-lifecycle',
        name: 'test-lifecycle',
        path: '/workspace',
        codeExecution: {
          type: 'docker',
          envVars: { TEST_VAR: 'test-value' },
          setupCommands: ['echo "setup complete"'],
        },
        useGitWorktrees: false,
      })

      const portMapping = await dockerService.allocateWorkerPort()

      // Create container
      await dockerService.createContainer(
        containerName,
        'task-lifecycle',
        mockRepo,
        tempWorkspace,
        portMapping,
        'alpine:latest',
      )

      // Verify container exists
      const container = await dockerService.getExistingContainer(containerName)
      expect(container).toBeTruthy()
      expect(container!.id).toBeTruthy()

      // Start container
      await dockerService.startContainer(containerName)

      // Check status after start
      const statusAfterStart = dockerService.getContainerStatus(containerName)
      expect(['initializing', 'ready']).toContain(statusAfterStart)

      // Stop container
      await dockerService.stopContainer(containerName)

      // Remove container
      await dockerService.removeContainer(containerName)

      // Verify container is gone
      const statusAfterRemove = dockerService.getContainerStatus(containerName)
      expect(statusAfterRemove).toBe('not_created')
    })
  })

  describe('Command Execution', () => {
    it('should execute commands in running container', async () => {
      const containerName = dockerService.generateContainerName('test-exec', 'task-exec')
      createdContainers.push(containerName)

      const mockRepo = createMockRepository({
        id: 'test-exec',
        name: 'test-exec',
        path: '/workspace',
        codeExecution: { type: 'docker', envVars: {}, setupCommands: [] },
        useGitWorktrees: false,
      })

      const portMapping = await dockerService.allocateWorkerPort()

      // Create and start container
      await dockerService.createContainer(
        containerName,
        'task-exec',
        mockRepo,
        tempWorkspace,
        portMapping,
        'alpine:latest',
      )

      await dockerService.startContainer(containerName)

      // Execute a simple command
      const result = await dockerService.executeCommandInContainer(
        containerName,
        'echo "Hello from container"',
        mockRepo,
        tempWorkspace,
        5000,
      )

      expect(result.stdout).toContain('Hello from container')
      expect(result.stderr).toBeDefined()
    })
  })

  describe('Error Handling', () => {
    it('should handle non-existent container operations gracefully', async () => {
      const nonExistentContainer = 'non-existent-container-12345'

      // These should not throw but handle gracefully
      await dockerService.stopContainer(nonExistentContainer)
      await dockerService.removeContainer(nonExistentContainer)

      const container = await dockerService.getExistingContainer(nonExistentContainer)
      expect(container).toBeNull()
    })

    it('should handle invalid command execution', async () => {
      const containerName = dockerService.generateContainerName('test-error', 'task-error')
      createdContainers.push(containerName)

      const mockRepo = createMockRepository({
        id: 'test-error',
        name: 'test-error',
        path: '/workspace',
        codeExecution: { type: 'docker', envVars: {}, setupCommands: [] },
        useGitWorktrees: false,
      })

      const portMapping = await dockerService.allocateWorkerPort()

      await dockerService.createContainer(
        containerName,
        'task-error',
        mockRepo,
        tempWorkspace,
        portMapping,
        'alpine:latest',
      )

      await dockerService.startContainer(containerName)

      // Execute command that will fail
      await expect(
        dockerService.executeCommandInContainer(
          containerName,
          'nonexistent-command-12345',
          mockRepo,
          tempWorkspace,
          5000,
        ),
      ).rejects.toThrow()
    })
  })

  describe('Helper Methods', () => {
    it('should skip tests when Docker is not available', async () => {
      // This test validates the skipIfNoDocker helper
      try {
        await dockerService.skipIfNoDocker()
        // If we reach here, Docker is available (which is expected in this test suite)
        expect(true).toBe(true)
      } catch (error) {
        // If Docker is not available, the error message should be informative
        expect(error instanceof Error).toBe(true)
        expect((error as Error).message).toMatch(/Skipping test:/)
      }
    })
  })
})
