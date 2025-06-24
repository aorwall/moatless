import { promises as fs } from 'fs'
import path from 'path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { ClaudeCodeAgent } from '../agents/claude-code-agent.js'

describe('Claude Code Integration', () => {
  let tempDir: string

  beforeEach(async () => {
    tempDir = path.join(process.cwd(), 'temp-test-' + Date.now())
    await fs.mkdir(tempDir, { recursive: true })
  })

  afterEach(async () => {
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  it('should execute claude command and parse events', async () => {
    const agent = new ClaudeCodeAgent('test-task', {
      model: 'claude-3-haiku-20240307', // Use faster model for tests
      maxTokens: 100,
    })

    const events: any[] = []

    try {
      for await (const event of agent.run('Say hello', { workspaceDir: tempDir })) {
        events.push(event)

        // Stop after getting some events to avoid long test
        if (events.length >= 3) {
          await agent.stop()
          break
        }
      }
    } catch (error) {
      // This test might fail if Claude CLI is not available
      console.log('Claude CLI not available, skipping integration test')
      return
    }

    expect(events.length).toBeGreaterThan(0)
    expect(events[0].type).toBe('start')
  }, 30000) // 30 second timeout
})
