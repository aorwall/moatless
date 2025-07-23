import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { RepositoryConfig } from '@moatless/types'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

// Helper function to create complete RepositoryConfig for tests
function createMockRepository(overrides: Partial<RepositoryConfig>): RepositoryConfig {
  return {
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    ...overrides,
  } as RepositoryConfig
}

import { createClaudeService } from '../../agents/claude-code/claude.service.js'
import { createDockerService, type DockerService } from '../../services/docker.service.js'
import { createEventService } from '../../services/event.service.js'
import { getGitHubService } from '../../services/github.service.js'
import { createSettingsService } from '../../services/settings.service.js'
import { createStorageService } from '../../services/storage.js'
import { DockerEnvironment } from '../docker-environment.js'

/**
 * Integration tests for DockerEnvironment + DockerService + ClaudeService
 *
 * These tests verify that DockerEnvironment correctly integrates with its dependencies
 * through dependency injection. They test the complete DockerEnvironment workflow
 * with real Docker operations but mocked Claude operations where appropriate.
 *
 * Prerequisites:
 * - Docker must be installed and running
 * - User must have permissions to run Docker commands
 * - CLAUDE_CODE_OAUTH_TOKEN environment variable should be set
 * - moatless-vibe-universal:latest image should be available (will fall back to alpine)
 */
describe('DockerEnvironment Integration Tests', () => {
  let dockerService: DockerService
  let claudeService: any
  let eventService: any
  let dockerEnvironment: DockerEnvironment
  let tempWorkspace: string
  let createdContainers: string[] = []

  beforeAll(async () => {
    // Create services following DI pattern
    const githubService = getGitHubService()
    const settingsService = await createSettingsService(githubService)
    const storageService = await createStorageService()

    dockerService = createDockerService(settingsService)
    claudeService = createClaudeService(settingsService)
    eventService = createEventService(storageService)

    // Check if Docker is available and skip tests if not
    const dockerCheck = await dockerService.checkDockerAvailable()
    if (!dockerCheck.available) {
      console.log(`Skipping DockerEnvironment integration tests: ${dockerCheck.message}`)
      return
    }

    // Create temporary workspace directory
    tempWorkspace = await fs.mkdtemp(path.join(os.tmpdir(), 'docker-env-test-'))

    // Create a basic test file in workspace
    await fs.writeFile(path.join(tempWorkspace, 'test-file.txt'), 'Hello Docker Environment!')

    // Create a basic git repository structure
    await fs.mkdir(path.join(tempWorkspace, '.git'), { recursive: true })
    await fs.writeFile(
      path.join(tempWorkspace, '.git', 'config'),
      '[core]\n    repositoryformatversion = 0',
    )
  })

  afterEach(async () => {
    // Clean up docker environment
    if (dockerEnvironment) {
      try {
        await dockerEnvironment.teardown(true)
      } catch {
        // Environment might not be set up or already torn down
      }
    }

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

  describe('Environment Creation and Setup', () => {
    it('should create DockerEnvironment with proper DI', () => {
      const mockRepo = createMockRepository({
        id: 'test-create',
        name: 'test-create',
        path: '/workspace',
        codeExecution: {
          type: 'docker',
          envVars: { NODE_ENV: 'test' },
          setupCommands: ['echo "Setting up test environment"'],
        },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-create-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      expect(dockerEnvironment).toBeDefined()
      expect(dockerEnvironment.getType()).toBe('docker')
      expect(dockerEnvironment.getStatus()).toBe('not_created')
      expect(dockerEnvironment.getWorkspacePath()).toBe(tempWorkspace)
    })

    it('should setup environment and create container', async () => {
      const mockRepo = createMockRepository({
        id: 'test-setup',
        name: 'test-setup',
        path: '/workspace',
        codeExecution: {
          type: 'docker',
          envVars: { NODE_ENV: 'test' },
          setupCommands: ['echo "Setup command executed"'],
        },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-setup-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      const containerName = dockerEnvironment.getContainerName()
      createdContainers.push(containerName)

      // Mock Claude authentication to avoid requiring real Claude setup
      vi.spyOn(claudeService, 'validateClaudeAuthentication').mockReturnValue({
        isValid: true,
        hasConfig: true,
        hasCredentials: true,
        hasOAuthToken: true,
      })

      vi.spyOn(claudeService, 'getClaudeConfigPaths').mockReturnValue({
        configPath: null,
        credentialsPath: null,
        claudeConfigDir: '/tmp/.claude',
      })

      // Setup the environment
      await dockerEnvironment.setup()

      expect(dockerEnvironment.getStatus()).toBe('ready')
      expect(dockerEnvironment.getContainerName()).toMatch(/moatless-test-setup-task-setup-test/)
      expect(dockerEnvironment.getPort()).toBeGreaterThan(8000)
      expect(dockerEnvironment.getImage()).toMatch(/moatless-vibe-universal:latest|alpine:latest/)
    }, 180000) // 3 minute timeout for container setup

    it('should handle setup failure gracefully', async () => {
      const mockRepo = createMockRepository({
        id: 'test-failure',
        name: 'test-failure',
        path: '/workspace',
        codeExecution: {
          type: 'docker',
          envVars: {},
          setupCommands: [],
        },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-failure-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      const containerName = dockerEnvironment.getContainerName()
      createdContainers.push(containerName)

      // Mock Claude authentication failure
      vi.spyOn(claudeService, 'validateClaudeAuthentication').mockReturnValue({
        isValid: false,
        hasConfig: true,
        hasCredentials: false,
        hasOAuthToken: false,
        error: 'No Claude credentials found - Claude Code agent will not work',
      })

      // This should succeed with warning since it's not a critical error
      await dockerEnvironment.setup()
      expect(dockerEnvironment.getStatus()).toBe('ready')
    }, 180000)
  })

  describe('Command Execution', () => {
    beforeEach(async () => {
      const mockRepo = createMockRepository({
        id: 'test-commands',
        name: 'test-commands',
        path: '/workspace',
        codeExecution: { type: 'docker', envVars: {}, setupCommands: [] },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-commands-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      const containerName = dockerEnvironment.getContainerName()
      createdContainers.push(containerName)

      // Mock Claude for setup
      vi.spyOn(claudeService, 'validateClaudeAuthentication').mockReturnValue({
        isValid: true,
        hasConfig: false,
        hasCredentials: false,
        hasOAuthToken: true,
      })

      vi.spyOn(claudeService, 'getClaudeConfigPaths').mockReturnValue({
        configPath: null,
        credentialsPath: null,
        claudeConfigDir: '/tmp/.claude',
      })

      await dockerEnvironment.setup()
    })

    it('should execute commands in container', async () => {
      const result = await dockerEnvironment.runCommand('echo "Hello from DockerEnvironment"')

      expect(result.exitCode).toBe(0)
      expect(result.output).toContain('Hello from DockerEnvironment')
      expect(result.command).toBe('echo "Hello from DockerEnvironment"')
      expect(result.durationMs).toBeGreaterThan(0)
    })

    it('should handle command failures', async () => {
      const result = await dockerEnvironment.runCommand('nonexistent-command-12345')

      expect(result.exitCode).not.toBe(0)
      expect(result.output).toBeTruthy()
      expect(result.command).toBe('nonexistent-command-12345')
    })

    it('should handle file operations', async () => {
      // Write a file
      const writeResult = await dockerEnvironment.runCommand(
        'echo "test content" > /tmp/test-file.txt',
      )
      expect(writeResult.exitCode).toBe(0)

      // Read the file
      const readResult = await dockerEnvironment.runCommand('cat /tmp/test-file.txt')
      expect(readResult.exitCode).toBe(0)
      expect(readResult.output).toContain('test content')
    })
  })

  describe('Agent Management', () => {
    beforeEach(async () => {
      const mockRepo = createMockRepository({
        id: 'test-agents',
        name: 'test-agents',
        path: '/workspace',
        codeExecution: { type: 'docker', envVars: {}, setupCommands: [] },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-agents-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      const containerName = dockerEnvironment.getContainerName()
      createdContainers.push(containerName)

      // Mock Claude for setup
      vi.spyOn(claudeService, 'validateClaudeAuthentication').mockReturnValue({
        isValid: true,
        hasConfig: false,
        hasCredentials: false,
        hasOAuthToken: true,
      })

      vi.spyOn(claudeService, 'getClaudeConfigPaths').mockReturnValue({
        configPath: null,
        credentialsPath: null,
        claudeConfigDir: '/tmp/.claude',
      })

      await dockerEnvironment.setup()
    })

    it('should create proxy agents', async () => {
      const agent = await dockerEnvironment.createAgent('claude-code')

      expect(agent).toBeDefined()
      expect(agent.constructor.name).toBe('ProxyAgent')
    })

    it('should handle agent creation with options', async () => {
      const agent = await dockerEnvironment.createAgent('claude-code', { model: 'claude-3-sonnet' })

      expect(agent).toBeDefined()
    })
  })

  describe('Environment Lifecycle', () => {
    it('should handle complete environment lifecycle', async () => {
      const mockRepo = createMockRepository({
        id: 'test-lifecycle',
        name: 'test-lifecycle',
        path: '/workspace',
        codeExecution: {
          type: 'docker',
          envVars: { TEST_MODE: 'integration' },
          setupCommands: ['echo "Lifecycle test setup"'],
        },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-lifecycle-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      const containerName = dockerEnvironment.getContainerName()
      createdContainers.push(containerName)

      // Mock Claude
      vi.spyOn(claudeService, 'validateClaudeAuthentication').mockReturnValue({
        isValid: true,
        hasConfig: false,
        hasCredentials: false,
        hasOAuthToken: true,
      })

      vi.spyOn(claudeService, 'getClaudeConfigPaths').mockReturnValue({
        configPath: null,
        credentialsPath: null,
        claudeConfigDir: '/tmp/.claude',
      })

      // Initial state
      expect(dockerEnvironment.getStatus()).toBe('not_created')

      // Setup
      await dockerEnvironment.setup()
      expect(dockerEnvironment.getStatus()).toBe('ready')

      // Run command
      const result = await dockerEnvironment.runCommand('echo "Environment is ready"')
      expect(result.exitCode).toBe(0)

      // Create agent
      const agent = await dockerEnvironment.createAgent('claude-code')
      expect(agent).toBeDefined()

      // Teardown
      await dockerEnvironment.teardown()

      // Container should be cleaned up
      const finalStatus = dockerService.getContainerStatus(containerName)
      expect(finalStatus).toBe('not_created')
    }, 300000) // 5 minute timeout for full lifecycle
  })

  describe('Error Recovery', () => {
    it('should recover from failed container state', async () => {
      const mockRepo = createMockRepository({
        id: 'test-recovery',
        name: 'test-recovery',
        path: '/workspace',
        codeExecution: { type: 'docker', envVars: {}, setupCommands: [] },
        useGitWorktrees: false,
      })

      dockerEnvironment = new DockerEnvironment(
        'task-recovery-test',
        mockRepo,
        tempWorkspace,
        dockerService,
        claudeService,
        eventService,
      )

      const containerName = dockerEnvironment.getContainerName()
      createdContainers.push(containerName)

      // Mock Claude
      vi.spyOn(claudeService, 'validateClaudeAuthentication').mockReturnValue({
        isValid: true,
        hasConfig: false,
        hasCredentials: false,
        hasOAuthToken: true,
      })

      vi.spyOn(claudeService, 'getClaudeConfigPaths').mockReturnValue({
        configPath: null,
        credentialsPath: null,
        claudeConfigDir: '/tmp/.claude',
      })

      // Setup environment
      await dockerEnvironment.setup()
      expect(dockerEnvironment.getStatus()).toBe('ready')

      // Manually stop the container to simulate failure
      await dockerService.stopContainer(containerName)

      // Environment should detect the issue and recover
      const result = await dockerEnvironment.runCommand('echo "Recovery test"')
      expect(result.exitCode).toBe(0)
      expect(dockerEnvironment.getStatus()).toBe('ready')
    }, 300000)
  })
})
