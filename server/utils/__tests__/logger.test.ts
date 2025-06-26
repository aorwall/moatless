import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createRequestLogger, createTaskLogger, logger } from '../logger.js'

// Mock pino
vi.mock('pino', () => {
  const mockLogger = {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    child: vi.fn(),
    level: 'info',
  }

  // Setup child to return a new mock logger with same methods
  mockLogger.child.mockImplementation(() => ({
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    child: vi.fn(),
    level: 'info',
  }))

  return {
    default: vi.fn(() => mockLogger),
  }
})

describe('Logger', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('logging methods', () => {
    it('should log info messages', () => {
      logger.info('Test message', { foo: 'bar' })
      expect(logger.info).toHaveBeenCalledWith('Test message', { foo: 'bar' })
    })

    it('should log error messages', () => {
      const error = new Error('Test error')
      logger.error('Error occurred', error)
      expect(logger.error).toHaveBeenCalledWith('Error occurred', error)
    })

    it('should log warn messages', () => {
      logger.warn('Warning message')
      expect(logger.warn).toHaveBeenCalledWith('Warning message')
    })

    it('should log debug messages', () => {
      logger.debug('Debug info', { debug: true })
      expect(logger.debug).toHaveBeenCalledWith('Debug info', { debug: true })
    })
  })

  describe('helper functions', () => {
    it('should create request-scoped logger', () => {
      const requestLogger = createRequestLogger('req-123', 'task-456')

      expect(logger.child).toHaveBeenCalledWith({
        requestId: 'req-123',
        taskId: 'task-456',
      })
      expect(requestLogger).toBeDefined()
    })

    it('should create task-scoped logger', () => {
      const taskLogger = createTaskLogger('task-789')

      expect(logger.child).toHaveBeenCalledWith({
        taskId: 'task-789',
      })
      expect(taskLogger).toBeDefined()
    })

    it('should create request logger without taskId', () => {
      const requestLogger = createRequestLogger('req-999')

      expect(logger.child).toHaveBeenCalledWith({
        requestId: 'req-999',
        taskId: undefined,
      })
      expect(requestLogger).toBeDefined()
    })
  })
})
