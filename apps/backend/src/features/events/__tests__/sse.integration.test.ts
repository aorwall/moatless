import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { build } from '../../../test/helper'

describe.skipIf(!process.env['DATABASE_URL'])('SSE Routes (Deprecated)', () => {
  let app: Awaited<ReturnType<typeof build>>

  beforeEach(async () => {
    app = await build()
  })

  afterEach(async () => {
    // Hono doesn't need cleanup
  })

  describe('Task-specific SSE endpoints (removed)', () => {
    it('should return 404 for removed task stream endpoint', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/tasks/any-task/stream', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(404)
    })
  })
})
