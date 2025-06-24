import type { FastifyError, FastifyReply, FastifyRequest } from 'fastify'
import { MoatlessError } from '../../shared/constants/errors.js'

export async function errorHandler(
  error: FastifyError | MoatlessError,
  request: FastifyRequest,
  reply: FastifyReply,
) {
  if (error instanceof MoatlessError) {
    return reply.status(error.statusCode).send({
      error: {
        code: error.code,
        message: error.message,
        details: error.details,
      },
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
    })
  }

  // Log unexpected errors
  request.log.error(error)

  return reply.status(500).send({
    error: {
      code: 'INTERNAL_SERVER_ERROR',
      message: 'An unexpected error occurred',
    },
  })
}
