import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'
import { ClaudeService } from '../../agents/claude-code/claude.service'

describe('ClaudeService Integration', () => {
  let service: ClaudeService
  let hasClaudeData: boolean

  beforeAll(async () => {
    // @ts-expect-error - container not available in test context
    // biome-ignore lint/correctness/noUndeclaredVariables: using global DI container in integration test
    const settingsService = container.resolve(SettingsService)
    service = new ClaudeService(settingsService)

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
  })

  it('should be able to get usage data with custom plan', async () => {
    const usage = await service.getUsage('custom')

    expect(usage).toBeDefined()
    expect(usage.cost).toBeDefined()
    expect(usage.tokens).toBeDefined()
    expect(usage.messageUsage).toBeDefined()
    expect(usage.timeToReset).toBeDefined()
    expect(usage.modelDistribution).toBeDefined()

    // Validate structure
    expect(typeof usage.cost.current).toBe('number')
    expect(typeof usage.cost.max).toBe('number')
    expect(typeof usage.tokens.current).toBe('number')
    expect(typeof usage.tokens.max).toBe('number')
    expect(typeof usage.messageUsage.current).toBe('number')
    expect(typeof usage.messageUsage.max).toBe('number')
    expect(typeof usage.timeToReset.startTime).toBe('string')
    expect(typeof usage.timeToReset.endTime).toBe('string')
    expect(typeof usage.modelDistribution).toBe('object')

    // Validate numbers are non-negative
    expect(usage.cost.current).toBeGreaterThanOrEqual(0)
    expect(usage.cost.max).toBeGreaterThanOrEqual(0)
    expect(usage.tokens.current).toBeGreaterThanOrEqual(0)
    expect(usage.tokens.max).toBeGreaterThanOrEqual(0)
    expect(usage.messageUsage.current).toBeGreaterThanOrEqual(0)
    expect(usage.messageUsage.max).toBeGreaterThanOrEqual(0)

    // Validate time format
    expect(() => new Date(usage.timeToReset.startTime)).not.toThrow()
    expect(() => new Date(usage.timeToReset.endTime)).not.toThrow()

    const startTime = new Date(usage.timeToReset.startTime)
    const endTime = new Date(usage.timeToReset.endTime)
    expect(endTime.getTime()).toBeGreaterThan(startTime.getTime())

    console.log('Claude Usage Data:', JSON.stringify(usage, null, 2))

    if (hasClaudeData) {
      console.log('✅ Real Claude usage data found and processed successfully!')
      console.log(`Current cost: ${usage.cost.current}`)
      console.log(`Current tokens: ${usage.tokens.current.toLocaleString()}`)
      console.log(`Current messages: ${usage.messageUsage.current}`)
      console.log('Model distribution:', usage.modelDistribution)
    } else {
      console.log('ℹ️  No Claude usage data found on this machine - returning empty state')
      // When no data is available, should return zero values
      expect(usage.cost.current).toBe(0)
      expect(usage.tokens.current).toBe(0)
      expect(usage.messageUsage.current).toBe(0)
    }
  })

  it('should work with different plans', async () => {
    const plans = ['pro', 'max5', 'max20', 'custom'] as const

    for (const plan of plans) {
      const usage = await service.getUsage(plan)

      expect(usage).toBeDefined()
      console.log(`Plan: ${plan}`)
      console.log(`  Max cost: ${usage.cost.max}`)
      console.log(`  Max tokens: ${usage.tokens.max.toLocaleString()}`)
      console.log(`  Max messages: ${usage.messageUsage.max}`)

      // Validate plan-specific limits
      switch (plan) {
        case 'pro':
          expect(usage.cost.max).toBe(18)
          expect(usage.tokens.max).toBe(19_000)
          expect(usage.messageUsage.max).toBe(1000)
          break
        case 'max5':
          expect(usage.cost.max).toBe(35)
          expect(usage.tokens.max).toBe(88_000)
          expect(usage.messageUsage.max).toBe(5000)
          break
        case 'max20':
          expect(usage.cost.max).toBe(140)
          expect(usage.tokens.max).toBe(220_000)
          expect(usage.messageUsage.max).toBe(20000)
          break
        case 'custom':
          // Custom plan uses P90 calculation or defaults
          expect(usage.cost.max).toBeGreaterThanOrEqual(0)
          expect(usage.tokens.max).toBeGreaterThanOrEqual(0)
          expect(usage.messageUsage.max).toBeGreaterThanOrEqual(0)
          break
      }
    }
  })

  it('should handle model distribution correctly', async () => {
    const usage = await service.getUsage('custom')

    expect(usage.modelDistribution).toBeDefined()

    // Sum of percentages should equal 100 (or 0 if no data)
    const totalPercentage = Object.values(usage.modelDistribution).reduce(
      (sum, pct) => sum + pct,
      0,
    )

    if (hasClaudeData && usage.tokens.current > 0) {
      expect(totalPercentage).toBeCloseTo(100, 1) // Allow 0.1% tolerance

      // Each percentage should be between 0 and 100
      for (const percentage of Object.values(usage.modelDistribution)) {
        expect(percentage).toBeGreaterThanOrEqual(0)
        expect(percentage).toBeLessThanOrEqual(100)
      }

      console.log('Model distribution validation passed:', usage.modelDistribution)
    } else {
      expect(totalPercentage).toBe(0)
      console.log('No active session found - empty model distribution is correct')
    }
  })

  it('should validate time reset window is 5 hours', async () => {
    const usage = await service.getUsage('custom')

    const startTime = new Date(usage.timeToReset.startTime)
    const endTime = new Date(usage.timeToReset.endTime)

    const diffMs = endTime.getTime() - startTime.getTime()
    const diffHours = diffMs / (1000 * 60 * 60)

    expect(diffHours).toBeCloseTo(5, 0.1) // Allow small tolerance for rounding

    console.log(
      `Time window: ${diffHours} hours (${startTime.toISOString()} to ${endTime.toISOString()})`,
    )
  })

  it('should handle cost calculations correctly', async () => {
    const usage = await service.getUsage('custom')

    // Cost should be reasonable (not negative, not extremely high)
    expect(usage.cost.current).toBeGreaterThanOrEqual(0)
    expect(usage.cost.current).toBeLessThan(1000) // Sanity check - less than $1000

    if (hasClaudeData && usage.cost.current > 0) {
      console.log(`Cost calculation validation passed: ${usage.cost.current}`)

      // Cost should be proportional to token usage
      const costPerToken = usage.cost.current / usage.tokens.current
      expect(costPerToken).toBeGreaterThan(0)
      expect(costPerToken).toBeLessThan(0.001) // Should be much less than $0.001 per token

      console.log(`Cost per token: ${costPerToken.toFixed(6)}`)
    }
  })
})
