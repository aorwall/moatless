#!/usr/bin/env tsx
/**
 * Standalone script to test Claude usage service
 */

import { getClaudeService } from '../agents/claude-code/claude.service.js'

async function testUsageService() {
  console.log('🧪 Testing Claude Usage Service...\n')
  console.log('📌 Note: Now using corrected token limits that match Python implementation\n')

  const now = new Date()
  console.log(`⏰ Current time: ${now.toLocaleTimeString()} (${now.toISOString()})\n`)

  const service = getClaudeService()

  const plans = ['custom', 'pro', 'max5', 'max20'] as const

  for (const plan of plans) {
    try {
      console.log(`📊 Testing plan: ${plan}`)
      const usage = await service.getUsage(plan)

      const costPercent =
        usage.cost.max > 0 ? ((usage.cost.current / usage.cost.max) * 100).toFixed(1) : '0.0'
      const tokenPercent =
        usage.tokens.max > 0 ? ((usage.tokens.current / usage.tokens.max) * 100).toFixed(1) : '0.0'
      const messagePercent =
        usage.messageUsage.max > 0
          ? ((usage.messageUsage.current / usage.messageUsage.max) * 100).toFixed(1)
          : '0.0'

      console.log(
        `  💰 Cost: $${usage.cost.current.toFixed(2)} / $${usage.cost.max} (${costPercent}%)`,
      )
      console.log(
        `  🎯 Tokens: ${usage.tokens.current.toLocaleString()} / ${usage.tokens.max.toLocaleString()} (${tokenPercent}%)`,
      )
      console.log(
        `  💬 Messages: ${usage.messageUsage.current} / ${usage.messageUsage.max} (${messagePercent}%)`,
      )

      // Calculate time to reset
      const endTime = new Date(usage.timeToReset.endTime)
      const timeToResetMs = endTime.getTime() - now.getTime()
      const hoursToReset = Math.floor(timeToResetMs / (1000 * 60 * 60))
      const minutesToReset = Math.floor((timeToResetMs % (1000 * 60 * 60)) / (1000 * 60))

      console.log(`  ⏰ Time window: ${usage.timeToReset.startTime} → ${usage.timeToReset.endTime}`)
      console.log(`  ⏱️  Time to reset: ${hoursToReset}h ${minutesToReset}m`)

      if (Object.keys(usage.modelDistribution).length > 0) {
        console.log('  🤖 Model distribution:')
        for (const [model, percentage] of Object.entries(usage.modelDistribution)) {
          const shortModel = model.includes('sonnet')
            ? 'Sonnet'
            : model.includes('opus')
              ? 'Opus'
              : model
          console.log(`    ${shortModel}: ${percentage}%`)
        }
      }

      console.log('')
    } catch (error) {
      console.error(`❌ Error testing ${plan} plan:`, error)
    }
  }

  console.log('✅ Testing complete!')
  console.log('\n📝 Changes from previous version:')
  console.log('  - Token limits corrected (divided by ~50-250x to match Python)')
  console.log('  - Cost limits fixed for max5 ($35) and max20 ($140)')
  console.log('  - Default custom limit: 44,000 tokens (was 2M)')
  console.log('  - File reading now defaults to all files (not just 24h)')
}

testUsageService().catch(console.error)
