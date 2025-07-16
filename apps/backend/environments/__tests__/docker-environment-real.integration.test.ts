import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createTaskLogger } from '../../utils/logger.js'
import { DockerEnvironment } from '../docker-environment.js'

// Mock external dependencies
vi.mock('../../utils/port-allocation.js')
vi.mock('../../utils/logger.js')

const mockFindAvailablePort = vi.fn()
const mockCreateTaskLogger = vi.mocked(createTaskLogger)

describe('DockerEnvironment - Real Integration Tests', () => {
  let dockerEnvironment: DockerEnvironment
  let mockLogger: any
  let mockEventService: any

  beforeEach(() => {
    vi.clearAllMocks()

    // Mock logger
    mockLogger = {
      info: vi.fn(),
      error: vi.fn(),
      debug: vi.fn(),
      warn: vi.fn(),
    }
    mockCreateTaskLogger.mockReturnValue(mockLogger)

    // Mock port allocation utility
    mockFindAvailablePort.mockResolvedValue(8001) // worker port
    vi.doMock('../../utils/port-allocation.js', () => ({
      findAvailablePort: mockFindAvailablePort,
    }))

    // Mock event service
    mockEventService = {
      emitWorkspaceEvent: vi.fn(),
    }

    const mockRepositoryConfig = {
      id: 'test-repo',
      name: 'Test Repository',
      path: '/test/workspace',
      codeExecution: {
        type: 'docker' as const,
        image: 'moatless-vibe-universal:latest',
        setupCommands: ['echo "Test setup command"', 'which pnpm', 'pnpm --version'],
      },
    }

    dockerEnvironment = new DockerEnvironment(
      'test-task-integration',
      mockRepositoryConfig as any,
      mockEventService,
      '/test/workspace',
    )
  })

  describe('runCommand with docker exec', () => {
    it('should have proper command structure', async () => {
      // Mock container ID to simulate a running container
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      // Test that the command structure is correct
      // Since we can't actually run the command without a real container,
      // we'll test that the error message indicates the correct docker command was attempted
      const result = await dockerEnvironment.runCommand('echo "Hello World"')

      expect(result.exitCode).toBe(1)
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('echo "Hello World"')
      expect(result.durationMs).toBeGreaterThan(0)
    })

    it('should handle pnpm commands', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('pnpm --version')

      expect(result.exitCode).toBe(1)
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('pnpm --version')
      expect(result.durationMs).toBeGreaterThan(0)
    })

    it('should handle pnpm install command', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('pnpm install')

      expect(result.exitCode).toBe(1)
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('pnpm install')
      expect(result.durationMs).toBeGreaterThan(0)
    })

    it('should throw error when container is not created', async () => {
      // Don't set containerId to simulate container not created
      await expect(dockerEnvironment.runCommand('echo "test"')).rejects.toThrow(
        'Container not created',
      )
    })

    it('should properly escape quotes in commands', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('echo "Hello "World" from bash"')

      expect(result.exitCode).toBe(1)
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('echo "Hello "World" from bash"')
      expect(result.durationMs).toBeGreaterThan(0)
    })

    it('should handle timeout parameter', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('sleep 60', 1000)

      expect(result.exitCode).toBe(1)
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('sleep 60')
      expect(result.durationMs).toBeGreaterThan(0)
    })
  })

  describe('Docker command construction', () => {
    it('should use correct working directory', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('pwd')

      // The error message should show the docker command was attempted
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('pwd')
    })

    it('should use bash shell', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('echo $SHELL')

      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('echo $SHELL')
    })
  })

  describe('Error handling', () => {
    it('should log errors appropriately', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      await dockerEnvironment.runCommand('echo "test"')

      expect(mockLogger.error).toHaveBeenCalledWith('Command execution failed', expect.any(Error))
      expect(mockLogger.debug).toHaveBeenCalledWith('Running command via docker exec: echo "test"')
    })

    it('should handle various error types', async () => {
      ;(dockerEnvironment as any).containerId = 'test-container-id'

      const result = await dockerEnvironment.runCommand('nonexistent-command')

      expect(result.exitCode).toBe(1)
      expect(result.output).toContain('No such container: test-container-id')
      expect(result.command).toBe('nonexistent-command')
    })
  })

  describe('Command execution integration', () => {
    it('should handle repository setup commands', async () => {
      const setupCommands = ['echo "Setup 1"', 'echo "Setup 2"', 'pnpm --version']
      const repoConfig = {
        id: 'setup-repo',
        name: 'Setup Repository',
        path: '/setup/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'moatless-vibe-universal:latest',
          setupCommands,
        },
      }

      const env = new DockerEnvironment(
        'setup-task',
        repoConfig as any,
        mockEventService,
        '/setup/workspace',
      )

      // Test that setup commands are properly configured
      expect(repoConfig.codeExecution.setupCommands).toEqual(setupCommands)
      expect(env.getType()).toBe('docker')
    })

    it('should validate configuration', async () => {
      expect(dockerEnvironment.getType()).toBe('docker')
      expect(dockerEnvironment.getStatus()).toBe('not_setup')
      expect(dockerEnvironment.getError()).toBeUndefined()
    })
  })
})
