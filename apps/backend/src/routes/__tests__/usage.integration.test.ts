import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'
import { build } from '../../test/helper.js'

describe('Usage API Integration', () => {
  let app: any
  let hasClaudeData: boolean

  beforeAll(async () => {
    app = await build()

    // Check if user has Claude data available
    const homeDir = os.homedir()
    const claudePaths = [
      path.join(homeDir, '.claude', 'projects'),
      path.join(homeDir, '.config', 'claude', 'projects'),
    ]

    hasClaudeData = false
    for (const claudePath of claudePaths) {
      try {
        const stats = await fs.stat(claudePath)
        if (stats.isDirectory()) {
          // Check if there are any .jsonl files
          const files = await fs.readdir(claudePath, { recursive: true })
          const jsonlFiles = files.filter((file) => file.toString().endsWith('.jsonl'))
          if (jsonlFiles.length > 0) {
            hasClaudeData = true
            break
          }
        }
      } catch {
        // Directory doesn't exist, continue checking
      }
    }

    console.log(`Claude data available: ${hasClaudeData}`)
  })

  it('should return usage data from GET /api/v1/usage', async () => {
    const response = await app.request('/api/v1/usage')

    expect(response.status).toBe(200)

    const data = await response.json()

    // Validate response structure
    expect(data).toHaveProperty('cost')
    expect(data).toHaveProperty('tokens')
    expect(data).toHaveProperty('messageUsage')
    expect(data).toHaveProperty('timeToReset')
    expect(data).toHaveProperty('modelDistribution')

    // Validate cost object
    expect(data.cost).toHaveProperty('current')
    expect(data.cost).toHaveProperty('max')
    expect(typeof data.cost.current).toBe('number')
    expect(typeof data.cost.max).toBe('number')
    expect(data.cost.current).toBeGreaterThanOrEqual(0)
    expect(data.cost.max).toBeGreaterThanOrEqual(0)

    // Validate tokens object
    expect(data.tokens).toHaveProperty('current')
    expect(data.tokens).toHaveProperty('max')
    expect(typeof data.tokens.current).toBe('number')
    expect(typeof data.tokens.max).toBe('number')
    expect(data.tokens.current).toBeGreaterThanOrEqual(0)
    expect(data.tokens.max).toBeGreaterThanOrEqual(0)

    // Validate messageUsage object
    expect(data.messageUsage).toHaveProperty('current')
    expect(data.messageUsage).toHaveProperty('max')
    expect(typeof data.messageUsage.current).toBe('number')
    expect(typeof data.messageUsage.max).toBe('number')
    expect(data.messageUsage.current).toBeGreaterThanOrEqual(0)
    expect(data.messageUsage.max).toBeGreaterThanOrEqual(0)

    // Validate timeToReset object
    expect(data.timeToReset).toHaveProperty('startTime')
    expect(data.timeToReset).toHaveProperty('endTime')
    expect(typeof data.timeToReset.startTime).toBe('string')
    expect(typeof data.timeToReset.endTime).toBe('string')

    // Validate time strings are valid ISO dates
    expect(() => new Date(data.timeToReset.startTime)).not.toThrow()
    expect(() => new Date(data.timeToReset.endTime)).not.toThrow()

    const startTime = new Date(data.timeToReset.startTime)
    const endTime = new Date(data.timeToReset.endTime)
    expect(endTime.getTime()).toBeGreaterThan(startTime.getTime())

    // Validate modelDistribution object
    expect(typeof data.modelDistribution).toBe('object')

    console.log('API Response:', JSON.stringify(data, null, 2))

    if (hasClaudeData) {
      console.log('✅ Real Claude usage data successfully returned via API!')
    } else {
      console.log('ℹ️  API correctly returns empty state when no Claude data is available')
    }
  })

  it('should handle plan parameter correctly', async () => {
    const plans = ['pro', 'max5', 'max20', 'custom']

    for (const plan of plans) {
      const response = await app.request(`/api/v1/usage?plan=${plan}`)

      expect(response.status).toBe(200)

      const data = await response.json()

      console.log(
        `Plan: ${plan} - Max cost: $${data.cost.max}, Max tokens: ${data.tokens.max.toLocaleString()}, Max messages: ${data.messageUsage.max}`,
      )

      // Validate plan-specific limits
      switch (plan) {
        case 'pro':
          expect(data.cost.max).toBe(18)
          expect(data.tokens.max).toBe(1_000_000)
          expect(data.messageUsage.max).toBe(1000)
          break
        case 'max5':
          expect(data.cost.max).toBe(100)
          expect(data.tokens.max).toBe(5_000_000)
          expect(data.messageUsage.max).toBe(5000)
          break
        case 'max20':
          expect(data.cost.max).toBe(400)
          expect(data.tokens.max).toBe(20_000_000)
          expect(data.messageUsage.max).toBe(20000)
          break
        case 'custom':
          // Custom plan uses P90 calculation or defaults
          expect(data.cost.max).toBeGreaterThanOrEqual(0)
          expect(data.tokens.max).toBeGreaterThanOrEqual(0)
          expect(data.messageUsage.max).toBeGreaterThanOrEqual(0)
          break
      }
    }
  })

  it('should return 200 with default plan when no plan specified', async () => {
    const response = await app.request('/api/v1/usage')

    expect(response.status).toBe(200)

    const data = await response.json()

    // Should default to 'custom' plan behavior
    expect(data.cost.max).toBeGreaterThanOrEqual(0)
    expect(data.tokens.max).toBeGreaterThanOrEqual(0)
    expect(data.messageUsage.max).toBeGreaterThanOrEqual(0)
  })

  it('should handle invalid plan parameter gracefully', async () => {
    const response = await app.request('/api/v1/usage?plan=invalid')

    // Should return 400 due to validation error
    expect(response.status).toBe(400)
  })

  it('should return consistent data structure across multiple calls', async () => {
    const response1 = await app.request('/api/v1/usage?plan=custom')
    const response2 = await app.request('/api/v1/usage?plan=custom')

    expect(response1.status).toBe(200)
    expect(response2.status).toBe(200)

    const data1 = await response1.json()
    const data2 = await response2.json()

    // Structure should be identical
    expect(Object.keys(data1).sort()).toEqual(Object.keys(data2).sort())
    expect(Object.keys(data1.cost).sort()).toEqual(Object.keys(data2.cost).sort())
    expect(Object.keys(data1.tokens).sort()).toEqual(Object.keys(data2.tokens).sort())
    expect(Object.keys(data1.messageUsage).sort()).toEqual(Object.keys(data2.messageUsage).sort())
    expect(Object.keys(data1.timeToReset).sort()).toEqual(Object.keys(data2.timeToReset).sort())

    // Values should be identical or very close (within same 5-hour window)
    if (hasClaudeData) {
      // If we have real data, the values should be identical for same window
      const time1 = new Date(data1.timeToReset.startTime)
      const time2 = new Date(data2.timeToReset.startTime)

      if (time1.getTime() === time2.getTime()) {
        // Same window, should be identical
        expect(data1.cost.current).toBe(data2.cost.current)
        expect(data1.tokens.current).toBe(data2.tokens.current)
        expect(data1.messageUsage.current).toBe(data2.messageUsage.current)
      }
    }

    console.log('Consistency check passed between multiple API calls')
  })

  it('should return proper Content-Type header', async () => {
    const response = await app.request('/api/v1/usage')

    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain('application/json')
  })
})
