import { beforeEach, describe, expect, it, vi } from 'vitest'

// Mock the logger module
vi.mock('../logger.js', () => {
  const mockChild = vi.fn()
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
      info: vi.fn(),
      error: vi.fn(),
      warn: vi.fn(),
      debug: vi.fn(),
      child: mockChild,
    },
  }
})

// Import after mocking
import { logger } from '../logger'

describe('Logger', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('logging methods', () => {
    it('should log info messages', () => {
      logger.info({ foo: 'bar' }, 'Test message')
      expect(logger.info).toHaveBeenCalledWith({ foo: 'bar' }, 'Test message')
    })

    it('should log error messages', () => {
      const error = new Error('Test error')
      logger.error({ err: error }, 'Error occurred')
      expect(logger.error).toHaveBeenCalledWith({ err: error }, 'Error occurred')
    })

    it('should log warn messages', () => {
      logger.warn({ debug: true }, 'Warning message')
      expect(logger.warn).toHaveBeenCalledWith({ debug: true }, 'Warning message')
    })

    it('should log debug messages', () => {
      logger.debug({ debug: true }, 'Debug info')
      expect(logger.debug).toHaveBeenCalledWith({ debug: true }, 'Debug info')
    })
  })

  describe('helper functions', () => {
    it('should create task-scoped logger', () => {
      const taskLogger = logger.child({ taskId: 'task-789' })

      expect(logger.child).toHaveBeenCalledWith({
        taskId: 'task-789',
      })
      expect(taskLogger).toBeDefined()
    })
  })
})
