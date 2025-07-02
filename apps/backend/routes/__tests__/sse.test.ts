import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

describe('SSE Routes (Deprecated)', () => {
  let app: Awaited<ReturnType<typeof build>>

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    await app.close()
  })

  describe('Task-specific SSE endpoints (removed)', () => {
    it('should return 404 for removed task stream endpoint', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/any-task/stream',
      })

      expect(response.statusCode).toBe(404)
    })

    it('should return 404 for removed task stream status endpoint', async () => {
      const response = await app.inject({
        method: 'GET',
        url: '/api/v1/tasks/any-task/stream/status',
      })

      expect(response.statusCode).toBe(404)
    })
  })

  // Note: Task-specific SSE endpoints have been removed in favor of the global event stream.
  // Real-time events are now delivered through the global /api/v1/events/stream endpoint
  // which provides events for all tasks with optional filtering.
})
