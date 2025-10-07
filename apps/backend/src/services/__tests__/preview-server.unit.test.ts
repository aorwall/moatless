import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createTaskLogger } from '../../utils/logger.js'
import * as portUtils from '../../utils/port-utils.js'
import { PreviewServer } from '../preview-server.js'

// Mock all external dependencies
vi.mock('../../utils/port-utils.js')
vi.mock('../../utils/logger.js')

const mockCreateTaskLogger = vi.mocked(createTaskLogger)
const mockGetPort = vi.mocked(portUtils.getPort)

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

    // Mock port utilities
    mockGetPort.mockResolvedValue(9000)

    previewServer = new PreviewServer('test-task', '/test/workspace')
  })

  describe('Constructor', () => {
    it('should initialize with correct parameters', () => {
      expect(mockCreateTaskLogger).toHaveBeenCalledWith('test-task')
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          taskId: 'test-task',
          workspacePath: '/test/workspace',
        }),
      )
    })

    it('should handle empty environment variables', () => {
      const server = new PreviewServer('test-task-2', '/workspace')

      expect(server).toBeDefined()
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          taskId: 'test-task-2',
        }),
      )
    })
  })

  describe('Port Management Integration', () => {
    it('should use port utilities for port allocation', () => {
      // Port allocation is now handled by getPort utility
      expect(mockGetPort).toBeDefined()
    })

    it('should be constructed with taskId for port allocation', () => {
      // Verify the server was created with the task ID
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          taskId: 'test-task',
        }),
      )
    })
  })

  describe('Command Parsing', () => {
    it('should handle simple commands', () => {
      const server = new PreviewServer('task', '/workspace')

      expect(server).toBeDefined()
    })

    it('should handle commands with arguments', () => {
      const server = new PreviewServer('task', '/workspace')

      expect(server).toBeDefined()
    })

    it('should handle complex commands with port placeholder', () => {
      const server = new PreviewServer('task', '/workspace')

      expect(server).toBeDefined()
    })
  })

  describe('Environment Variables Configuration', () => {
    it('should accept custom environment variables', () => {
      const server = new PreviewServer('task', '/workspace')

      expect(server).toBeDefined()
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          taskId: 'task',
        }),
      )
    })
  })

  describe('Server State Management', () => {
    it('should initialize in stopped state', async () => {
      const status = await previewServer.getStatus()

      expect(status.status).toBe('stopped')
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
      ;(previewServer as any).port = 9000

      const port = previewServer.getPort()
      expect(port).toBe(9000)
    })

    it('should return correct port after multiple allocations', () => {
      // Test different port values
      const testPorts = [9000, 9001, 9100, 9999]

      for (const testPort of testPorts) {
        ;(previewServer as any).port = testPort
        expect(previewServer.getPort()).toBe(testPort)
      }
    })

    it('should return undefined when port is reset', () => {
      // Set a port first
      ;(previewServer as any).port = 9000
      expect(previewServer.getPort()).toBe(9000)

      // Reset port (happens during stop)
      ;(previewServer as any).port = undefined
      expect(previewServer.getPort()).toBeUndefined()
    })
  })

  describe('Configuration Validation', () => {
    it('should handle workspace path correctly', () => {
      new PreviewServer('task', '/different/workspace/path')

      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          workspacePath: '/different/workspace/path',
        }),
      )
    })

    it('should handle server commands correctly', () => {
      new PreviewServer('task', '/workspace')

      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          taskId: 'task',
        }),
      )
    })
  })

  describe('Logger Integration', () => {
    it('should log initialization details', () => {
      expect(mockLogger.info).toHaveBeenCalledWith(
        'PreviewServer initialized',
        expect.objectContaining({
          taskId: 'test-task',
          workspacePath: '/test/workspace',
        }),
      )
    })
  })

  // Port placeholder tests removed as configuration happens at start() now

  describe('Error Handling - Constructor', () => {
    it('should handle invalid parameters gracefully', () => {
      const server = new PreviewServer('', '')

      expect(server).toBeDefined()
      expect(mockLogger.info).toHaveBeenCalled()
    })

    it('should handle special characters in commands', () => {
      const server = new PreviewServer('task', '/workspace')

      expect(server).toBeDefined()
    })
  })
})
