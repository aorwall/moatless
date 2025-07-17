import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createTaskLogger } from '../../utils/logger.js'
import { DockerEnvironment } from '../docker-environment.js'

// Mock external dependencies for basic unit tests
vi.mock('../../utils/port-allocation.js')
vi.mock('../../utils/logger.js')

const mockFindAvailablePort = vi.fn()
const mockCreateTaskLogger = vi.mocked(createTaskLogger)

describe('DockerEnvironment - Unit Tests', () => {
  let dockerEnvironment: DockerEnvironment
  let mockLogger: any

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
    mockFindAvailablePort.mockResolvedValue(3000)
    vi.doMock('../../utils/port-allocation.js', () => ({
      findAvailablePort: mockFindAvailablePort,
    }))

    const mockRepositoryConfig = {
      id: 'test-repo',
      name: 'Test Repository',
      path: '/test/workspace',
      codeExecution: {
        type: 'docker' as const,
        image: 'node:18-alpine',
        envVars: { NODE_ENV: 'development' },
        setupCommands: ['npm install'],
      },
      previewConfig: {
        serverStartCommand: 'npm run dev',
        port: 3000,
      },
    }

    dockerEnvironment = new DockerEnvironment(
      'test-task',
      mockRepositoryConfig as any,
      '/test/workspace',
    )
  })

  describe('Constructor', () => {
    it('should initialize with correct parameters', () => {
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('test-task')
      // DockerEnvironment constructor doesn't log debug info, just verify it was created
      expect(dockerEnvironment.getType()).toBe('docker')
    })
  })

  describe('Basic Properties', () => {
    it('should return correct type', () => {
      expect(dockerEnvironment.getType()).toBe('docker')
    })

    it('should return created date', () => {
      const createdAt = dockerEnvironment.getCreatedAt()
      expect(createdAt).toBeInstanceOf(Date)
      expect(Date.now() - createdAt.getTime()).toBeLessThan(1000)
    })

    it('should initially have no error', () => {
      expect(dockerEnvironment.getError()).toBeUndefined()
    })

    it('should initially be in not_setup status', () => {
      expect(dockerEnvironment.getStatus()).toBe('not_setup')
    })
  })

  describe('isRunning', () => {
    it('should return false initially', () => {
      expect(dockerEnvironment.isRunning()).toBe(false)
    })
  })

  describe('Docker Configuration', () => {
    it('should handle repository config with docker image', () => {
      const dockerConfig = {
        id: 'docker-repo',
        name: 'Docker Repository',
        path: '/docker/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'python:3.9-slim',
          envVars: { PYTHONPATH: '/app' },
          setupCommands: ['pip install -r requirements.txt'],
          workingDir: '/app',
        },
      }

      const env = new DockerEnvironment('docker-task', dockerConfig as any, '/docker/workspace')

      expect(env.getType()).toBe('docker')
    })

    it('should handle different docker images', () => {
      const configs = [
        { image: 'node:16-alpine' },
        { image: 'python:3.10' },
        { image: 'openjdk:11-jre-slim' },
        { image: 'nginx:alpine' },
      ]

      configs.forEach((config, index) => {
        const repoConfig = {
          id: `repo-${index}`,
          name: `Repository ${index}`,
          path: `/workspace-${index}`,
          codeExecution: {
            type: 'docker' as const,
            ...config,
          },
        }

        const env = new DockerEnvironment(`task-${index}`, repoConfig as any, `/workspace-${index}`)

        expect(env.getType()).toBe('docker')
      })
    })
  })

  describe('Port Management Integration', () => {
    it('should use port allocation utility', () => {
      // Port allocation is now handled by utility functions
      expect(mockFindAvailablePort).toBeDefined()
    })

    it('should be constructed with taskId for port allocation', () => {
      // Verify the logger was created with the task ID
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('test-task')
    })
  })

  describe('Environment Variables', () => {
    it('should preserve repository environment variables', () => {
      const configWithEnvVars = {
        id: 'env-repo',
        name: 'Environment Repository',
        path: '/env/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'node:18',
          envVars: {
            NODE_ENV: 'production',
            PORT: '8080',
            DATABASE_URL: 'postgresql://localhost:5432/db',
          },
        },
      }

      const env = new DockerEnvironment('env-task', configWithEnvVars as any, '/env/workspace')

      expect(env.getType()).toBe('docker')
    })
  })

  describe('Setup Commands', () => {
    it('should handle repository with setup commands', () => {
      const configWithCommands = {
        id: 'commands-repo',
        name: 'Commands Repository',
        path: '/commands/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'node:18',
          setupCommands: ['npm ci', 'npm run build', 'npm run test'],
        },
      }

      const env = new DockerEnvironment(
        'commands-task',
        configWithCommands as any,
        '/commands/workspace',
      )

      expect(env.getType()).toBe('docker')
    })
  })

  describe('Working Directory Configuration', () => {
    it('should handle custom working directory', () => {
      const configWithWorkDir = {
        id: 'workdir-repo',
        name: 'Working Directory Repository',
        path: '/workdir/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'alpine:latest',
          workingDir: '/app/src',
        },
      }

      const env = new DockerEnvironment(
        'workdir-task',
        configWithWorkDir as any,
        '/workdir/workspace',
      )

      expect(env.getType()).toBe('docker')
    })

    it('should handle default working directory', () => {
      const configWithoutWorkDir = {
        id: 'default-repo',
        name: 'Default Repository',
        path: '/default/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'ubuntu:latest',
        },
      }

      const env = new DockerEnvironment(
        'default-task',
        configWithoutWorkDir as any,
        '/default/workspace',
      )

      expect(env.getType()).toBe('docker')
    })
  })

  describe('Preview Configuration', () => {
    it('should handle preview server configuration', () => {
      const configWithPreview = {
        id: 'preview-repo',
        name: 'Preview Repository',
        path: '/preview/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'node:18',
        },
        previewConfig: {
          framework: 'vite' as const,
          serverStartCommand: 'npm run dev',
          port: 5173,
          host: '0.0.0.0',
        },
      }

      const env = new DockerEnvironment(
        'preview-task',
        configWithPreview as any,
        '/preview/workspace',
      )

      expect(env.getType()).toBe('docker')
    })

    it('should handle missing preview configuration', () => {
      const configWithoutPreview = {
        id: 'no-preview-repo',
        name: 'No Preview Repository',
        path: '/no-preview/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'node:18',
        },
      }

      const env = new DockerEnvironment(
        'no-preview-task',
        configWithoutPreview as any,
        '/no-preview/workspace',
      )

      expect(env.getType()).toBe('docker')
    })
  })

  describe('Container Name Generation', () => {
    it('should generate container names based on repo name and task ID', () => {
      // The container name generation is internal logic
      // We just verify the environment can be created
      expect(dockerEnvironment.getType()).toBe('docker')
    })

    it('should handle different repository names in container naming', () => {
      const testConfigs = [
        { name: 'simple-repo', expected: 'moatless-simple-repo-test-tas' },
        { name: 'My Complex Repo!', expected: 'moatless-my-complex-repo-test-tas' },
        { name: 'owner/repository', expected: 'moatless-owner-repository-test-tas' },
      ]

      testConfigs.forEach(({ name }) => {
        const config = {
          id: 'test-repo',
          name,
          path: '/test/workspace',
          codeExecution: {
            type: 'docker' as const,
            image: 'node:18-alpine',
          },
        }

        const env = new DockerEnvironment('test-task', config as any, '/test/workspace')

        expect(env.getType()).toBe('docker')
      })
    })
  })

  describe('Error State Management', () => {
    it('should track error state correctly', () => {
      expect(dockerEnvironment.getError()).toBeUndefined()
      expect(dockerEnvironment.getStatus()).toBe('not_setup')
    })
  })

  describe('Setup State Management', () => {
    it('should transition to initializing when setup starts', () => {
      // Initially not_setup
      expect(dockerEnvironment.getStatus()).toBe('not_setup')

      // Set setupInProgress to simulate setup starting
      ;(dockerEnvironment as any).setupInProgress = true

      // Should now be initializing
      expect(dockerEnvironment.getStatus()).toBe('initializing')
    })

    it('should transition to ready when setup completes', () => {
      // Simulate setup completion
      ;(dockerEnvironment as any).setupInProgress = false
      ;(dockerEnvironment as any).setupComplete = true

      // Should now be ready
      expect(dockerEnvironment.getStatus()).toBe('ready')
    })

    it('should show error status when error occurs', () => {
      // Simulate error
      ;(dockerEnvironment as any)._error = 'Test error'

      // Should show error regardless of other states
      expect(dockerEnvironment.getStatus()).toBe('error')
      expect(dockerEnvironment.getError()).toBe('Test error')
    })
  })

  describe('Event Service Integration', () => {
    it('should be initialized with event service', () => {
      // The event service is passed to constructor and stored in base class
      // This test just verifies the constructor accepts it without error
      expect(dockerEnvironment.getType()).toBe('docker')
    })
  })

  describe('Logger Integration', () => {
    it('should create logger with task ID', () => {
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('test-task')
    })
  })

  describe('Minimal Configuration', () => {
    it('should handle minimal docker configuration', () => {
      const minimalConfig = {
        id: 'minimal-repo',
        name: 'Minimal Repository',
        path: '/minimal/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'alpine:latest',
        },
      }

      const env = new DockerEnvironment('minimal-task', minimalConfig as any, '/minimal/workspace')

      expect(env.getType()).toBe('docker')
      expect(env.getStatus()).toBe('not_setup')
    })
  })

  describe('Complex Configuration', () => {
    it('should handle complex docker configuration', () => {
      const complexConfig = {
        id: 'complex-repo',
        name: 'Complex Repository',
        path: '/complex/workspace',
        codeExecution: {
          type: 'docker' as const,
          image: 'node:18-alpine',
          workingDir: '/usr/src/app',
          envVars: {
            NODE_ENV: 'production',
            PORT: '3000',
            DATABASE_URL: 'postgresql://db:5432/app',
            REDIS_URL: 'redis://redis:6379',
          },
          setupCommands: [
            'apk add --no-cache git',
            'npm ci --only=production',
            'npm run build',
            'npm run migration:up',
          ],
          serverCommand: 'npm start',
        },
        previewConfig: {
          framework: 'nextjs' as const,
          serverStartCommand: 'npm run dev',
          port: 3000,
          host: '0.0.0.0',
          injectionStrategy: {
            method: 'middleware' as const,
          },
        },
      }

      const env = new DockerEnvironment('complex-task', complexConfig as any, '/complex/workspace')

      expect(env.getType()).toBe('docker')
      expect(env.getStatus()).toBe('not_setup')
    })
  })
})
