#!/usr/bin/env node

import { getClaudeService } from '../agents/claude-code/claude.service.js'

async function testSessionUsage() {
  console.log('🧪 Testing Claude Session Usage Service...\n')

  const service = getClaudeService()

  // Test with a few session IDs
  const sessionIds = [
    'd13399b4-2c6b-4326-a602-2ae627afed37',
    '096d8ba9-f7db-403c-956d-35eb5cb4069e',
    '84b80c67-607d-4ff3-aaf5-4f9aca753451',
    'non-existent-session-id', // Test with a non-existent session
  ]

  for (const sessionId of sessionIds) {
    try {
      console.log(`📊 Testing session: ${sessionId}`)
      const usage = await service.getSessionUsage(sessionId)

      if (usage.messageCount === 0 && !usage.startTime) {
        console.log('  ❌ Session not found\n')
        continue
      }

      console.log(`  💰 Cost: $${usage.cost.toFixed(4)}`)
      console.log('  🎯 Tokens:')
      console.log(`     Input: ${usage.tokens.input_tokens.toLocaleString()}`)
      console.log(`     Output: ${usage.tokens.output_tokens.toLocaleString()}`)
      console.log(`     Cache Creation: ${usage.tokens.cache_creation_tokens.toLocaleString()}`)
      console.log(`     Cache Read: ${usage.tokens.cache_read_tokens.toLocaleString()}`)
      console.log(`     Total: ${usage.tokens.total_tokens.toLocaleString()}`)
      console.log(`  💬 Messages: ${usage.messageCount}`)

      if (usage.startTime && usage.endTime) {
        const duration = new Date(usage.endTime).getTime() - new Date(usage.startTime).getTime()
        const minutes = Math.floor(duration / 1000 / 60)
        console.log(`  ⏱️  Duration: ${minutes} minutes`)
        console.log(`  📅 Period: ${usage.startTime} → ${usage.endTime}`)
      }

      if (Object.keys(usage.modelDistribution).length > 0) {
        console.log('  🤖 Model distribution:')
        for (const [model, percentage] of Object.entries(usage.modelDistribution)) {
          console.log(`     ${model}: ${percentage}%`)
        }
      }

      console.log('')
    } catch (error) {
      console.error(`  ❌ Error: ${error instanceof Error ? error.message : 'Unknown error'}\n`)
    }
  }

  console.log('✅ Testing complete!')
}

testSessionUsage().catch(console.error)
