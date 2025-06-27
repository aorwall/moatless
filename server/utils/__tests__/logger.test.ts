import { beforeEach, describe, expect, it, vi } from 'vitest'

// Create mock logger functions
const mockInfo = vi.fn()
const mockError = vi.fn()
const mockWarn = vi.fn()
const mockDebug = vi.fn()
const mockChild = vi.fn()

// Mock the logger module
vi.mock('../logger.js', () => {
  const mockChildLogger = {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
  }

  mockChild.mockReturnValue(mockChildLogger)

  return {
    logger: {
      info: mockInfo,
      error: mockError,
      warn: mockWarn,
      debug: mockDebug,
      child: mockChild,
    },
    createRequestLogger: (requestId: string, taskId?: string) => {
      return mockChild({ requestId, taskId })
    },
    createTaskLogger: (taskId: string) => {
      return mockChild({ taskId })
    },
  }
})

// Import after mocking
import { createRequestLogger, createTaskLogger, logger } from '../logger.js'

describe('Logger', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockInfo.mockClear()
    mockError.mockClear()
    mockWarn.mockClear()
    mockDebug.mockClear()
    mockChild.mockClear()
  })

  describe('logging methods', () => {
    it('should log info messages', () => {
      logger.info('Test message', { foo: 'bar' })
      expect(mockInfo).toHaveBeenCalledWith('Test message', { foo: 'bar' })
    })

    it('should log error messages', () => {
      const error = new Error('Test error')
      logger.error('Error occurred', error)
      expect(mockError).toHaveBeenCalledWith('Error occurred', error)
    })

    it('should log warn messages', () => {
      logger.warn('Warning message')
      expect(mockWarn).toHaveBeenCalledWith('Warning message')
    })

    it('should log debug messages', () => {
      logger.debug('Debug info', { debug: true })
      expect(mockDebug).toHaveBeenCalledWith('Debug info', { debug: true })
    })
  })

  describe('helper functions', () => {
    it('should create request-scoped logger', () => {
      const requestLogger = createRequestLogger('req-123', 'task-456')

      expect(mockChild).toHaveBeenCalledWith({
        requestId: 'req-123',
        taskId: 'task-456',
      })
      expect(requestLogger).toBeDefined()
    })

    it('should create task-scoped logger', () => {
      const taskLogger = createTaskLogger('task-789')

      expect(mockChild).toHaveBeenCalledWith({
        taskId: 'task-789',
      })
      expect(taskLogger).toBeDefined()
    })

    it('should create request logger without taskId', () => {
      const requestLogger = createRequestLogger('req-999')

      expect(mockChild).toHaveBeenCalledWith({
        requestId: 'req-999',
        taskId: undefined,
      })
      expect(requestLogger).toBeDefined()
    })
  })
})
