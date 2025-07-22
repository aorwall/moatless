import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createTaskLogger } from '../../utils/logger.js'
import { PreviewServer } from '../preview-server.js'

// Mock all external dependencies
vi.mock('../../utils/port-allocation.js', () => ({
  findAvailablePort: vi.fn(),
}))
vi.mock('../../utils/logger.js')

const mockCreateTaskLogger = vi.mocked(createTaskLogger)

describe('PreviewServer - Unit Tests', () => {
  let previewServer: PreviewServer
  let mockLogger: any

  beforeEach(async () => {
    vi.clearAllMocks()

    // Mock logger
    mockLogger = {
      info: vi.fn(),
      error: vi.fn(),
      debug: vi.fn(),
      warn: vi.fn(),
    }
    mockCreateTaskLogger.mockReturnValue(mockLogger)

    // Mock port allocation utility - import after mocking
    const { findAvailablePort } = await import('../../utils/port-allocation.js')
    vi.mocked(findAvailablePort).mockResolvedValue(3000)

    previewServer = new PreviewServer('test-task', '/test/workspace', 'npm run dev', {
      NODE_ENV: 'development',
    })
  })

  describe('Constructor', () => {
    it('should initialize with correct parameters', () => {
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('preview-server')
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          taskId: 'test-task',
          workspacePath: '/test/workspace',
          serverCommand: 'npm run dev',
          envVars: ['NODE_ENV'],
        }),
      )
    })

    it('should handle empty environment variables', () => {
      const server = new PreviewServer('test-task-2', '/workspace', 'python manage.py runserver')

      expect(server).toBeDefined()
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          taskId: 'test-task-2',
          envVars: [],
        }),
      )
    })
  })

  describe('Port Management Integration', () => {
    it('should use port allocation utility', () => {
      // Port allocation is now handled by utility functions
      expect(mockFindAvailablePort).toBeDefined()
    })

    it('should be constructed with taskId for port allocation', () => {
      // Verify the server was created with the task ID
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          taskId: 'test-task',
        }),
      )
    })
  })

  describe('Command Parsing', () => {
    it('should handle simple commands', () => {
      const server = new PreviewServer('task', '/workspace', 'npm start')

      expect(server).toBeDefined()
    })

    it('should handle commands with arguments', () => {
      const server = new PreviewServer(
        'task',
        '/workspace',
        'python manage.py runserver --host=0.0.0.0',
      )

      expect(server).toBeDefined()
    })

    it('should handle complex commands with port placeholder', () => {
      const server = new PreviewServer(
        'task',
        '/workspace',
        'node server.js --port={PORT} --env=development',
      )

      expect(server).toBeDefined()
    })
  })

  describe('Environment Variables Configuration', () => {
    it('should accept custom environment variables', () => {
      const customEnvVars = {
        NODE_ENV: 'production',
        DEBUG: 'true',
        API_KEY: 'secret-key',
      }

      const server = new PreviewServer('task', '/workspace', 'npm start', customEnvVars)

      expect(server).toBeDefined()
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          envVars: ['NODE_ENV', 'DEBUG', 'API_KEY'],
        }),
      )
    })
  })

  describe('Server State Management', () => {
    it('should initialize in stopped state', async () => {
      const status = await previewServer.getStatus()

      expect(status.running).toBe(false)
      expect(status.url).toBeUndefined()
      expect(status.pid).toBeUndefined()
    })
  })

  describe('Port Management', () => {
    it('should return undefined port when server is not running', () => {
      const port = previewServer.getPort()
      expect(port).toBeUndefined()
    })

    it('should return port when server has allocated port', () => {
      // Simulate port allocation by setting internal port
      // This would normally happen during start()
      ;(previewServer as any).port = 3000

      const port = previewServer.getPort()
      expect(port).toBe(3000)
    })

    it('should return correct port after multiple allocations', () => {
      // Test different port values
      const testPorts = [3000, 3001, 4000, 8080]

      testPorts.forEach((testPort) => {
        ;(previewServer as any).port = testPort
        expect(previewServer.getPort()).toBe(testPort)
      })
    })

    it('should return undefined when port is reset', () => {
      // Set a port first
      ;(previewServer as any).port = 3000
      expect(previewServer.getPort()).toBe(3000)

      // Reset port (happens during stop)
      ;(previewServer as any).port = undefined
      expect(previewServer.getPort()).toBeUndefined()
    })
  })

  describe('Configuration Validation', () => {
    it('should handle workspace path correctly', () => {
      new PreviewServer('task', '/different/workspace/path', 'npm start')

      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          workspacePath: '/different/workspace/path',
        }),
      )
    })

    it('should handle server commands correctly', () => {
      new PreviewServer('task', '/workspace', 'yarn dev --port={PORT}')

      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          serverCommand: 'yarn dev --port={PORT}',
        }),
      )
    })
  })

  describe('Logger Integration', () => {
    it('should log initialization details', () => {
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          taskId: 'test-task',
          workspacePath: '/test/workspace',
          serverCommand: 'npm run dev',
          portPlaceholder: '{PORT}',
          envVars: ['NODE_ENV'],
        }),
      )
    })
  })

  describe('Port Placeholder Configuration', () => {
    it('should use correct port placeholder', () => {
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized with:',
        expect.objectContaining({
          portPlaceholder: '{PORT}',
        }),
      )
    })
  })

  describe('Error Handling - Constructor', () => {
    it('should handle invalid parameters gracefully', () => {
      const server = new PreviewServer('', '', '')

      expect(server).toBeDefined()
      expect(mockLogger.info).toHaveBeenCalled()
    })

    it('should handle special characters in commands', () => {
      const server = new PreviewServer(
        'task',
        '/workspace',
        'node -e "console.log(\'Hello World\')" --port={PORT}',
      )

      expect(server).toBeDefined()
    })
  })
})
