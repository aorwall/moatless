import type { FastifyError, FastifyReply, FastifyRequest } from 'fastify'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ERROR_CODES, MoatlessError } from '@moatless/shared/constants/errors'
import { errorHandler } from '../error-handler.js'

describe('errorHandler', () => {
  const mockRequest = {
    log: {
      error: vi.fn(),
    },
  } as unknown as FastifyRequest

  const mockReply = {
    status: vi.fn().mockReturnThis(),
    send: vi.fn().mockReturnThis(),
  } as unknown as FastifyReply

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should handle MoatlessError correctly', async () => {
    const error = new MoatlessError('Task not found', ERROR_CODES.TASK_NOT_FOUND, 404, {
      taskId: 'task-123',
    })

    await errorHandler(error, mockRequest, mockReply)

    expect(mockReply.status).toHaveBeenCalledWith(404)
    expect(mockReply.send).toHaveBeenCalledWith({
      error: {
        code: ERROR_CODES.TASK_NOT_FOUND,
        message: 'Task not found',
        details: { taskId: 'task-123' },
      },
    })
  })

  it('should handle Fastify validation errors', async () => {
    const error: FastifyError = {
      name: 'FastifyError',
      message: 'Validation failed',
      code: 'FST_ERR_VALIDATION',
      statusCode: 400,
      validation: [
        {
          keyword: 'required',
          instancePath: '.message',
          schemaPath: '#/properties/message/required',
          params: { missingProperty: 'message' },
          message: 'should have required property message',
        },
      ],
    }

    await errorHandler(error, mockRequest, mockReply)

    expect(mockReply.status).toHaveBeenCalledWith(400)
    expect(mockReply.send).toHaveBeenCalledWith({
      error: {
        code: 'VALIDATION_ERROR',
        message: 'Request validation failed',
        details: error.validation,
      },
    })
  })

  it('should handle unexpected errors', async () => {
    const error = new Error('Unexpected error') as FastifyError

    await errorHandler(error, mockRequest, mockReply)

    expect(mockRequest.log.error).toHaveBeenCalledWith(error)
    expect(mockReply.status).toHaveBeenCalledWith(500)
    expect(mockReply.send).toHaveBeenCalledWith({
      error: {
        code: 'INTERNAL_SERVER_ERROR',
        message: 'Internal server error',
      },
      requestId: undefined,
    })
  })

  it('should not expose internal error details in production', async () => {
    const error = new Error('Database connection failed') as FastifyError

    await errorHandler(error, mockRequest, mockReply)

    expect(mockReply.send).toHaveBeenCalledWith({
      error: {
        code: 'INTERNAL_SERVER_ERROR',
        message: 'Internal server error',
      },
      requestId: undefined,
    })
    // Should not include the actual error message
    expect(mockReply.send).not.toHaveBeenCalledWith(
      expect.objectContaining({
        error: expect.objectContaining({
          message: 'Database connection failed',
        }),
      }),
    )
  })
})
