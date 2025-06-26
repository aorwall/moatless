import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createRequestLogger, createTaskLogger, Logger } from '../logger.js'

// Mock pino
vi.mock('pino', () => {
  const mockPinoInstance = {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    fatal: vi.fn(),
    child: vi.fn(),
    level: 'info',
  }

  // Setup child to return self
  mockPinoInstance.child.mockReturnValue(mockPinoInstance)

  return {
    default: vi.fn(() => mockPinoInstance),
    destination: vi.fn(),
    multistream: vi.fn(),
    transport: vi.fn(),
  }
})

describe('Logger', () => {
  let logger: Logger
  let mockPinoLogger: any

  beforeEach(() => {
    vi.clearAllMocks()
    // Create logger without console logging to test pino integration
    logger = new Logger({ service: 'test', console: false })
    // Get the mock instance from the logger
    mockPinoLogger = (logger as any).logger
  })

  describe('logging methods', () => {
    it('should log trace messages', () => {
      logger.trace('Test trace', { foo: 'bar' })

      expect(mockPinoLogger.trace).toHaveBeenCalledWith(
        expect.objectContaining({ foo: 'bar' }),
        'Test trace',
      )
    })

    it('should log debug messages', () => {
      logger.debug('Test debug', { foo: 'bar' })

      expect(mockPinoLogger.debug).toHaveBeenCalledWith(
        expect.objectContaining({ foo: 'bar' }),
        'Test debug',
      )
    })

    it('should log info messages', () => {
      logger.info('Test info', { foo: 'bar' })

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({ foo: 'bar' }),
        'Test info',
      )
    })

    it('should log warn messages', () => {
      logger.warn('Test warn', { foo: 'bar' })

      expect(mockPinoLogger.warn).toHaveBeenCalledWith(
        expect.objectContaining({ foo: 'bar' }),
        'Test warn',
      )
    })

    it('should log error messages with error object', () => {
      const error = new Error('Test error')
      logger.error('Test error message', error, { foo: 'bar' })

      expect(mockPinoLogger.error).toHaveBeenCalledWith(
        expect.objectContaining({
          foo: 'bar',
          error: {
            message: 'Test error',
            stack: expect.any(String),
            name: 'Error',
          },
        }),
        'Test error message',
      )
    })

    it('should log fatal messages', () => {
      const error = new Error('Fatal error')
      logger.fatal('Fatal error occurred', error)

      expect(mockPinoLogger.fatal).toHaveBeenCalledWith(
        expect.objectContaining({
          error: {
            message: 'Fatal error',
            stack: expect.any(String),
            name: 'Error',
          },
        }),
        'Fatal error occurred',
      )
    })
  })

  describe('context management', () => {
    it('should set and include context in logs', () => {
      logger.setContext({ taskId: '123', userId: '456' })
      logger.info('Test with context')

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: '123',
          userId: '456',
        }),
        'Test with context',
      )
    })

    it('should clear context', () => {
      logger.setContext({ taskId: '123' })
      logger.clearContext()
      logger.info('Test without context')

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.not.objectContaining({
          taskId: '123',
        }),
        'Test without context',
      )
    })

    it('should create child logger with context', () => {
      logger.child({ requestId: 'req-123' })

      expect(mockPinoLogger.child).toHaveBeenCalledWith({ requestId: 'req-123' })
    })
  })

  describe('specialized logging methods', () => {
    it('should log HTTP requests', () => {
      const req = {
        method: 'GET',
        url: '/api/test',
        headers: { 'user-agent': 'test-agent' },
        ip: '127.0.0.1',
      }

      logger.logRequest(req)

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'GET',
          url: '/api/test',
          userAgent: 'test-agent',
          ip: '127.0.0.1',
        }),
        'HTTP Request',
      )
    })

    it('should log HTTP responses', () => {
      const req = { method: 'GET', url: '/api/test' }
      const res = { statusCode: 200 }

      logger.logResponse(req, res, 123.45)

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'GET',
          url: '/api/test',
          statusCode: 200,
          responseTime: '123.45ms',
        }),
        'HTTP Response',
      )
    })

    it('should log performance metrics', () => {
      logger.logPerformance('database.query', 45.67)

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: 'database.query',
          duration: '45.67ms',
        }),
        'Performance',
      )
    })

    it('should log task events', () => {
      logger.logTaskEvent('task-123', 'started', { agent: 'claude' })

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          event: 'started',
          agent: 'claude',
        }),
        'Task Event',
      )
    })

    it('should log agent events', () => {
      logger.logAgentEvent('task-123', 'claude-code', 'message.sent', {
        messageId: 'msg-456',
      })

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          agentType: 'claude-code',
          event: 'message.sent',
          messageId: 'msg-456',
        }),
        'Agent Event',
      )
    })

    it('should log workspace events', () => {
      logger.logWorkspaceEvent('task-123', 'local', 'created', {
        path: '/workspace/123',
      })

      expect(mockPinoLogger.info).toHaveBeenCalledWith(
        expect.objectContaining({
          taskId: 'task-123',
          workspaceType: 'local',
          event: 'created',
          path: '/workspace/123',
        }),
        'Workspace Event',
      )
    })
  })

  describe('helper functions', () => {
    it('should create request-scoped logger', () => {
      createRequestLogger('req-123', 'task-456')

      // The mock returns 'this' for child, so we just verify child was called
      expect(mockPinoLogger.child).toHaveBeenCalledWith({
        requestId: 'req-123',
        taskId: 'task-456',
      })
    })

    it('should create task-scoped logger', () => {
      createTaskLogger('task-789')

      expect(mockPinoLogger.child).toHaveBeenCalledWith({
        taskId: 'task-789',
      })
    })
  })
})
