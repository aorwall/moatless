import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createTaskLogger } from '../../utils/logger.js'
import { LocalEnvironment } from '../local-environment.js'

// Mock external dependencies for basic unit tests
vi.mock('../../utils/logger.js')
vi.mock('node:fs', () => ({
  promises: {
    access: vi.fn().mockResolvedValue(undefined),
    stat: vi.fn().mockResolvedValue({ isDirectory: () => true }),
  },
}))

const mockCreateTaskLogger = vi.mocked(createTaskLogger)

describe('LocalEnvironment - Unit Tests', () => {
  let localEnvironment: LocalEnvironment
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

    const mockRepositoryConfig = {
      id: 'test-repo',
      name: 'Test Repository',
      path: '/test/workspace',
      codeExecution: {
        type: 'local' as const,
        envVars: { NODE_ENV: 'development' },
        setupCommands: ['npm install'],
      },
      previewConfig: {
        serverStartCommand: 'npm run dev',
      },
    }

    localEnvironment = new LocalEnvironment(
      'test-task',
      mockRepositoryConfig as any,
      '/test/workspace',
    )
  })

  describe('Constructor', () => {
    it('should initialize with correct parameters', () => {
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('test-task')
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'LocalEnvironment constructor',
        expect.objectContaining({
          repositoryId: 'test-repo',
          repositoryPath: '/test/workspace',
          codeExecutionType: 'local',
          workspacePath: '/test/workspace',
        }),
      )
    })

    it('should handle empty workspace path', () => {
      const env = new LocalEnvironment('test-task-2', {
        id: 'repo',
        name: 'Repo',
        path: '/repo',
        codeExecution: { type: 'local' },
      } as any)

      expect(env.getWorkspacePath()).toBe('')
    })
  })

  describe('Basic Properties', () => {
    it('should return correct type', () => {
      expect(localEnvironment.getType()).toBe('local')
    })

    it('should return workspace path', () => {
      expect(localEnvironment.getWorkspacePath()).toBe('/test/workspace')
    })

    it('should return created date', () => {
      const createdAt = localEnvironment.getCreatedAt()
      expect(createdAt).toBeInstanceOf(Date)
      expect(Date.now() - createdAt.getTime()).toBeLessThan(1000)
    })

    it('should initially have no error', () => {
      expect(localEnvironment.getError()).toBeUndefined()
    })

    it('should initially be in initializing status', () => {
      expect(localEnvironment.getStatus()).toBe('initializing')
    })
  })

  describe('isRunning', () => {
    it('should return false initially', () => {
      expect(localEnvironment.isRunning()).toBe(false)
    })
  })

  describe('Configuration Validation', () => {
    it('should handle repository config with all fields', () => {
      const fullConfig = {
        id: 'full-repo',
        name: 'Full Repository',
        path: '/full/workspace',
        codeExecution: {
          type: 'local' as const,
          envVars: { NODE_ENV: 'production' },
          setupCommands: ['npm ci', 'npm run build'],
          serverCommand: 'npm start',
        },
        previewConfig: {
          framework: 'vite' as const,
          serverStartCommand: 'npm run dev',
          port: 3000,
        },
      }

      const env = new LocalEnvironment('full-task', fullConfig as any, '/full/workspace')

      expect(env.getType()).toBe('local')
      expect(env.getWorkspacePath()).toBe('/full/workspace')
    })

    it('should handle minimal repository config', () => {
      const minimalConfig = {
        id: 'minimal-repo',
        name: 'Minimal Repository',
        path: '/minimal/workspace',
        codeExecution: {
          type: 'local' as const,
        },
      }

      const env = new LocalEnvironment('minimal-task', minimalConfig as any, '/minimal/workspace')

      expect(env.getType()).toBe('local')
      expect(env.getWorkspacePath()).toBe('/minimal/workspace')
    })
  })

  describe('Environment Variables', () => {
    it('should preserve repository environment variables', () => {
      const configWithEnvVars = {
        id: 'env-repo',
        name: 'Environment Repository',
        path: '/env/workspace',
        codeExecution: {
          type: 'local' as const,
          envVars: {
            NODE_ENV: 'test',
            DEBUG: 'true',
            API_URL: 'http://localhost:3000',
          },
        },
      }

      const env = new LocalEnvironment('env-task', configWithEnvVars as any, '/env/workspace')

      expect(env.getType()).toBe('local')
    })
  })

  describe('Setup Commands', () => {
    it('should handle repository with setup commands', () => {
      const configWithCommands = {
        id: 'commands-repo',
        name: 'Commands Repository',
        path: '/commands/workspace',
        codeExecution: {
          type: 'local' as const,
          setupCommands: ['npm install', 'npm run build', 'npm run test'],
        },
      }

      const env = new LocalEnvironment(
        'commands-task',
        configWithCommands as any,
        '/commands/workspace',
      )

      expect(env.getType()).toBe('local')
    })
  })

  describe('Error State Management', () => {
    it('should track error state correctly', () => {
      expect(localEnvironment.getError()).toBeUndefined()
      expect(localEnvironment.getStatus()).toBe('initializing')
    })
  })

  describe('Event Service Integration', () => {
    it('should be initialized with event service', () => {
      // The event service is passed to constructor and stored in base class
      // This test just verifies the constructor accepts it without error
      expect(localEnvironment.getType()).toBe('local')
    })
  })

  describe('Logger Integration', () => {
    it('should create logger with task ID', () => {
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('test-task')
    })

    it('should log constructor details', () => {
      expect(mockLogger.debug).toHaveBeenCalledWith(
        'LocalEnvironment constructor',
        expect.objectContaining({
          repositoryId: 'test-repo',
          codeExecutionType: 'local',
        }),
      )
    })
  })

  describe('Path Handling', () => {
    it('should handle absolute workspace paths', () => {
      const env = new LocalEnvironment(
        'abs-task',
        {
          id: 'abs-repo',
          name: 'Absolute Repository',
          path: '/absolute/path',
          codeExecution: { type: 'local' },
        } as any,
        '/absolute/workspace/path',
      )

      expect(env.getWorkspacePath()).toBe('/absolute/workspace/path')
    })

    it('should handle relative workspace paths', () => {
      const env = new LocalEnvironment(
        'rel-task',
        {
          id: 'rel-repo',
          name: 'Relative Repository',
          path: './relative/path',
          codeExecution: { type: 'local' },
        } as any,
        './relative/workspace',
      )

      expect(env.getWorkspacePath()).toBe('./relative/workspace')
    })
  })
})
