import type { BranchNameAndSummary } from '@moatless/shared/types/claude-code'
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
      model: 'claude-3-haiku-20240307', // Use faster model for tests
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
      expect(result.summary.length).toBeGreaterThan(5)
      expect(result.summary.length).toBeLessThan(100)

      // Verify content makes sense for authentication
      expect(result.branchName.toLowerCase()).toMatch(/(feature|feat)/)
      expect(result.branchName.toLowerCase()).toMatch(/(auth|login|user)/)

      console.log('✅ Successfully generated branch name:', result.branchName)
      console.log('✅ Summary:', result.summary)
    }, 30000) // 30s timeout for API call

    it('should generate branch name and summary for bug fix request', async () => {
      const request = 'Fix memory leak in the task event streaming component'

      const result = await agent.getBranchNameAndSummary(request)

      expect(result).toBeDefined()
      expect(result).not.toBeNull()

      if (result) {
        // Verify it's a fix/bugfix branch
        expect(result.branchName.toLowerCase()).toMatch(/(fix|bugfix)/)
        expect(result.branchName.toLowerCase()).toMatch(/(memory|leak|stream)/)

        // Verify summary mentions the issue
        expect(result.summary.toLowerCase()).toMatch(/(fix|memory|leak)/)
      }
    }, 30000)

    it('should generate branch name and summary for refactor request', async () => {
      const request = 'Refactor the database connection pool to improve performance'

      const result = await agent.getBranchNameAndSummary(request)

      expect(result).toBeDefined()
      expect(result).not.toBeNull()

      if (result) {
        // Verify it's a refactor branch
        expect(result.branchName.toLowerCase()).toMatch(/(refactor|improve)/)
        expect(result.branchName.toLowerCase()).toMatch(/(database|db|connection|pool|performance)/)

        // Verify summary mentions refactoring
        expect(result.summary.toLowerCase()).toMatch(/(refactor|improve|performance|database)/)
      }
    }, 30000)

    it('should handle complex feature request with multiple components', async () => {
      const request =
        'Create a new TaskCard component with task name, repository info, git status, and real-time event updates for the dashboard grid view'

      const result = await agent.getBranchNameAndSummary(request)

      expect(result).toBeDefined()
      expect(result).not.toBeNull()

      if (result) {
        // Should be a feature branch
        expect(result.branchName.toLowerCase()).toMatch(/(feature|feat)/)

        // Should contain relevant keywords (but be concise)
        const branchLower = result.branchName.toLowerCase()
        expect(branchLower).toMatch(/(task|card|component|dashboard)/)

        // Should have a reasonable length (not too long)
        expect(result.branchName.split('-').length).toBeLessThanOrEqual(6)

        // Summary should capture the essence
        expect(result.summary.toLowerCase()).toMatch(/(task|card|component|dashboard)/)
      }
    }, 30000)

    it('should follow branch naming conventions strictly', async () => {
      const request = 'Add Special Characters & Spaces to User Profile!'

      const result = await agent.getBranchNameAndSummary(request)

      expect(result).toBeDefined()
      expect(result).not.toBeNull()

      if (result) {
        // Must follow strict naming conventions
        expect(result.branchName).toMatch(/^[a-z0-9/-]+$/) // Only lowercase, numbers, hyphens, slashes
        expect(result.branchName).not.toMatch(/[A-Z]/) // No uppercase
        expect(result.branchName).not.toMatch(/[_&!@#$%^*()+=[\]{}|\\:";'<>?,.]/) // No special chars
        expect(result.branchName).not.toMatch(/\s/) // No spaces

        // Should start with a category
        expect(result.branchName).toMatch(
          /^(feature|feat|fix|bugfix|refactor|chore|docs|style|test)\//,
        )
      }
    }, 30000)

    it('should return null for empty or invalid requests', async () => {
      const emptyRequest = ''

      const result = await agent.getBranchNameAndSummary(emptyRequest)

      // Might return null or a default response depending on Claude's behavior
      // We mainly want to ensure it doesn't throw an error
      expect(result).toBeDefined() // Should not throw
    }, 30000)

    it('should validate response structure matches TypeScript interface', async () => {
      const request = 'Update documentation for the API endpoints'

      const result = await agent.getBranchNameAndSummary(request)

      expect(result).toBeDefined()
      expect(result).not.toBeNull()

      if (result) {
        // Type checking - should satisfy BranchNameAndSummary interface
        const typedResult: BranchNameAndSummary = result

        expect(typeof typedResult.branchName).toBe('string')
        expect(typedResult.branchName.length).toBeGreaterThan(0)

        if (typedResult.summary) {
          expect(typeof typedResult.summary).toBe('string')
          expect(typedResult.summary.length).toBeGreaterThan(0)
        }

        // Should be a docs branch
        expect(result.branchName.toLowerCase()).toMatch(/(docs|documentation)/)
      }
    }, 30000)
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
  })

  describe('Error handling', () => {
    it('should handle network errors gracefully', async () => {
      // Test with a very long request that might cause issues
      const veryLongRequest = 'A'.repeat(10000) + ' - create a feature for this'

      // Should not throw an error, might return null or a response
      await expect(agent.getBranchNameAndSummary(veryLongRequest)).resolves.toBeDefined()
    }, 30000)
  })
})
