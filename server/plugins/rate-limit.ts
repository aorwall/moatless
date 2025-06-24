import rateLimit from '@fastify/rate-limit'
import type { FastifyInstance } from 'fastify'

export async function setupRateLimit(app: FastifyInstance) {
  await app.register(rateLimit, {
    max: 100, // 100 requests
    timeWindow: '1 minute', // per minute
    errorResponseBuilder: () => ({
      error: {
        code: 'RATE_LIMIT_EXCEEDED',
        message: 'Rate limit exceeded. Please try again later.',
      },
    }),
  })
}
