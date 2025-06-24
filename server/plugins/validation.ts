import { randomUUID } from 'node:crypto'
import type { FastifyInstance } from 'fastify'
import { ValidationError } from '../../shared/constants/errors.js'

export async function setupValidation(app: FastifyInstance) {
  // Custom schema compiler for Zod integration
  app.setValidatorCompiler(({ schema }) => {
    return (data) => {
      try {
        if ('safeParse' in schema && typeof schema.safeParse === 'function') {
          // Zod schema
          const result = schema.safeParse(data)
          if (result.success) {
            return { value: result.data }
          }
          return {
            error: new ValidationError('Validation failed', result.error.issues) as Error,
          }
        }
        // Fallback to default validation
        return { value: data }
      } catch (error) {
        return { error: error as Error }
      }
    }
  })

  // Add request ID to all requests
  app.addHook('onRequest', async (request, reply) => {
    const requestId = (request.headers['x-request-id'] as string) || randomUUID()
    request.id = requestId
    reply.header('x-request-id', requestId)
  })

  // Add response serialization options
  app.setSerializerCompiler(({ schema: _schema }) => {
    return (data) => {
      // Handle Date serialization
      return JSON.stringify(data, (_key, value) => {
        if (value instanceof Date) {
          return value.toISOString()
        }
        return value
      })
    }
  })
}
