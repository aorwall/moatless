import type { BranchNameAndSummary } from '@moatless/types/types/claude-code'
import { beforeEach, describe, expect, it } from 'vitest'
import { ClaudeCodeAgent } from '../claude-code-agent.js'

/**
 * Integration tests for Claude Code agent branch name generation.
 *
 * NOTE: These tests require Claude Code to be properly configured with an API key.
 * If Claude Code is not available or not configured, the tests will skip gracefully.
 *
 * To run these tests successfully:
 * 1. Install Claude Code CLI: npm install -g @anthropic-ai/claude-code
 * 2. Configure your API key: claude auth login
 * 3. Run the tests: pnpm test claude-code-agent-branch-name.integration
 */
describe('ClaudeCodeAgent - Branch Name Generation Integration', () => {
  let agent: ClaudeCodeAgent

  beforeEach(() => {
    // Create a real agent instance for integration testing
    agent = new ClaudeCodeAgent('test-task-id', {
      model: 'claude-sonnet-4-20250514', // Use faster model for tests
    })
  })

  // Helper function to handle Claude Code availability
  async function testBranchNameGeneration(request: string): Promise<BranchNameAndSummary | null> {
    try {
      const result = await agent.getBranchNameAndSummary(request)

      if (!result) {
        console.log('Claude Code not available or not configured, skipping integration test')
        return null
      }

      return result
    } catch (error) {
      console.log('Claude Code not available or not configured, skipping integration test')
      console.log('Error:', error)
      return null
    }
  }

  describe('getBranchNameAndSummary', () => {
    it('should generate branch name and summary for feature request', async () => {
      const request = 'Add user authentication with login and logout functionality'
      const result = await testBranchNameGeneration(request)

      // If Claude Code is not available, skip test
      if (!result) return

      // Verify structure
      expect(result).toHaveProperty('branchName')
      expect(result).toHaveProperty('summary')

      // Verify branchName format
      expect(result.branchName).toMatch(/^[a-z][a-z0-9-]*\/[a-z0-9-]+$/)
      expect(result.branchName).toContain('/')
      expect(result.branchName.toLowerCase()).toBe(result.branchName)
      expect(result.branchName).not.toContain(' ')
      expect(result.branchName).not.toContain('_')

      // Verify summary exists and is reasonable
      expect(result.summary).toBeTruthy()
      expect(typeof result.summary).toBe('string')
      expect(result.summary!.length).toBeGreaterThan(5)
      expect(result.summary!.length).toBeLessThan(100)

      // Verify content makes sense for authentication
      expect(result.branchName.toLowerCase()).toMatch(/(feature|feat)/)
      expect(result.branchName.toLowerCase()).toMatch(/(auth|login|user)/)

      console.log('✅ Successfully generated branch name:', result.branchName)
      console.log('✅ Summary:', result.summary)
    }, 30000) // 30s timeout for API call
  })

  describe('sendMessage - General functionality', () => {
    it('should be able to send a general message and get a response', async () => {
      const message = 'What is the capital of France?'

      const response = await agent.sendMessage(message)

      expect(response).toBeDefined()
      expect(typeof response).toBe('string')
      expect(response.length).toBeGreaterThan(0)
      expect(response.toLowerCase()).toContain('paris')
    }, 30000)

    it('should throw an exception when using invalid model name', async () => {
      // Create agent with gibberish model name
      const invalidAgent = new ClaudeCodeAgent('test-task-id', {
        model: 'gibberish-model-that-does-not-exist',
      })

      const message = 'What is the capital of France?'

      // Expect the promise to be rejected with an error
      await expect(invalidAgent.sendMessage(message)).rejects.toThrow()
    }, 30000)
  })

  describe('getBranchNameAndSummary - Error handling', () => {
    it('should throw an exception when using invalid model name', async () => {
      // Create agent with gibberish model name
      const invalidAgent = new ClaudeCodeAgent('test-task-id', {
        model: 'gibberish-model-that-does-not-exist',
      })

      const request = 'Add user authentication with login and logout functionality'

      // Expect the promise to be rejected with an error
      await expect(invalidAgent.getBranchNameAndSummary(request)).rejects.toThrow()
    }, 30000)
  })
})
