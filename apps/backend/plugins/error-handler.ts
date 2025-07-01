import type { FastifyError, FastifyReply, FastifyRequest } from 'fastify'
import { MoatlessError } from '@moatless/shared/constants/errors'

export async function errorHandler(
  error: FastifyError | MoatlessError,
  request: FastifyRequest,
  reply: FastifyReply,
) {
  const requestId = request.id

  if (error instanceof MoatlessError) {
    return reply.status(error.statusCode).send({
      error: {
        code: error.code,
        message: error.message,
        details: error.details,
      },
      requestId,
    })
  }

  // Handle Fastify validation errors
  if (error.validation) {
    return reply.status(400).send({
      error: {
        code: 'VALIDATION_ERROR',
        message: 'Request validation failed',
        details: error.validation,
      },
      requestId,
    })
  }

  // Log unexpected errors
  request.log.error(error)

  return reply.status(500).send({
    error: {
      code: 'INTERNAL_SERVER_ERROR',
      message: 'Internal server error',
    },
    requestId,
  })
}
