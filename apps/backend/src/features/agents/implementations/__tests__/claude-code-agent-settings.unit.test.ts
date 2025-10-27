import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// Mock the Claude Agent SDK
vi.mock('@anthropic-ai/claude-agent-sdk', async () => {
  const { createMockQueryImplementation } = await import(
    '../../../../test-utils/claude-agent-mocks.js'
  )
  return {
    query: createMockQueryImplementation(),
  }
})

// Mock logger
vi.mock('../../utils/logger.js', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
    debug: vi.fn(),
    child: vi.fn().mockReturnThis(),
  },
}))

// Mock calculateContextTokens
vi.mock('../../utils/context-tokens.js', () => ({
  calculateContextTokens: vi.fn().mockReturnValue(100),
}))

import { query as mockQuery } from '@anthropic-ai/claude-agent-sdk'
import { ClaudeCodeAgent } from '../claude-code/claude-code-agent'

describe('ClaudeCodeAgent Settings Integration', () => {
  let tempDir: string
  let originalEnv: NodeJS.ProcessEnv
  let agent: ClaudeCodeAgent

  beforeEach(async () => {
    // Save original environment
    originalEnv = { ...process.env }

    // Create temporary directory for tests
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-claude-settings-test-'))
    process.env['DATA_DIR'] = tempDir

    // Clear Claude-related env vars to ensure we test settings properly
    Reflect.deleteProperty(process.env, 'CLAUDE_BIN_DIR')
    Reflect.deleteProperty(process.env, 'CLAUDE_CONFIG_DIR')

    // Clear the mock before each test
    vi.mocked(mockQuery).mockClear()

    // Create agent with minimal options
    agent = new ClaudeCodeAgent('test-task-id', {
      model: 'claude-3-sonnet-20240320',
    })
  })

  afterEach(async () => {
    // Restore original environment - clear current env first
    for (const key in process.env) {
      if (!(key in originalEnv)) {
        Reflect.deleteProperty(process.env, key)
      }
    }
    // Then restore original values
    Object.assign(process.env, originalEnv)

    // Clean up temp directory
    await fs.rm(tempDir, { recursive: true, force: true })

    // Clear all mocks
    vi.clearAllMocks()
  })

  // Helper function to consume async generator
  async function runAgent(agent: ClaudeCodeAgent, message: string, options: any = {}) {
    const generator = agent.run(message, options)
    const events = []
    for await (const event of generator) {
      events.push(event)
    }
    return events
  }

  describe('Claude settings integration', () => {
    it('should not set environment variables when settings are empty', async () => {
      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalled()

      // Environment variables should not be set for empty strings
      expect(process.env['CLAUDE_BIN_DIR']).toBeUndefined()
      expect(process.env['CLAUDE_CONFIG_DIR']).toBeUndefined()
    })

    it('should set environment variables when settings are configured', async () => {
      // Create mock GitHub service
      const mockGitHubService = {
        apiBaseUrl: 'https://api.github.com',
        getAuthenticatedUser: vi.fn().mockResolvedValue(null),
        getUserEmails: vi.fn().mockResolvedValue(null),
        validateToken: vi.fn().mockResolvedValue(false),
        getTokenScopes: vi.fn().mockResolvedValue(null),
      } as any

      // Save settings with Claude configuration
      const { SettingsService } = await import('../../../../services/settings.service.js')
      const settingsService = new SettingsService(mockGitHubService)
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      })

      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalled()

      // Environment variables should be set from settings
      expect(process.env['CLAUDE_BIN_DIR']).toBe('/custom/bin')
      expect(process.env['CLAUDE_CONFIG_DIR']).toBe('/custom/config')
    })

    it('should set environment variables from env vars when no settings override', async () => {
      // Set environment variables
      process.env['CLAUDE_BIN_DIR'] = '/env/bin'
      process.env['CLAUDE_CONFIG_DIR'] = '/env/config'

      // Create mock GitHub service
      const mockGitHubService = {
        apiBaseUrl: 'https://api.github.com',
        getAuthenticatedUser: vi.fn().mockResolvedValue(null),
        getUserEmails: vi.fn().mockResolvedValue(null),
        validateToken: vi.fn().mockResolvedValue(false),
        getTokenScopes: vi.fn().mockResolvedValue(null),
      } as any

      // Reinitialize settings service to pick up env vars
      const { SettingsService } = await import('../../../../services/settings.service.js')
      const settingsService = new SettingsService(mockGitHubService)
      await settingsService.initialize()

      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalled()

      // Environment variables should be set from original env vars
      expect(process.env['CLAUDE_BIN_DIR']).toBe('/env/bin')
      expect(process.env['CLAUDE_CONFIG_DIR']).toBe('/env/config')
    })

    it('should prioritize settings over environment variables', async () => {
      // Set environment variables first
      process.env['CLAUDE_BIN_DIR'] = '/env/bin'
      process.env['CLAUDE_CONFIG_DIR'] = '/env/config'

      // Create mock GitHub service
      const mockGitHubService = {
        apiBaseUrl: 'https://api.github.com',
        getAuthenticatedUser: vi.fn().mockResolvedValue(null),
        getUserEmails: vi.fn().mockResolvedValue(null),
        validateToken: vi.fn().mockResolvedValue(false),
        getTokenScopes: vi.fn().mockResolvedValue(null),
      } as any

      // Initialize settings service and override with settings
      const { SettingsService } = await import('../../../../services/settings.service.js')
      const settingsService = new SettingsService(mockGitHubService)
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/settings/bin',
          configDir: '/settings/config',
        },
      })

      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalled()

      // Settings should override environment variables
      expect(process.env['CLAUDE_BIN_DIR']).toBe('/settings/bin')
      expect(process.env['CLAUDE_CONFIG_DIR']).toBe('/settings/config')
    })

    it('should handle partial Claude settings', async () => {
      // Create mock GitHub service
      const mockGitHubService = {
        apiBaseUrl: 'https://api.github.com',
        getAuthenticatedUser: vi.fn().mockResolvedValue(null),
        getUserEmails: vi.fn().mockResolvedValue(null),
        validateToken: vi.fn().mockResolvedValue(false),
        getTokenScopes: vi.fn().mockResolvedValue(null),
      } as any

      // Set only binDir in settings
      const { SettingsService } = await import('../../../../services/settings.service.js')
      const settingsService = new SettingsService(mockGitHubService)
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          // configDir is not set
        },
      })

      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalled()

      // Only binDir should be set
      expect(process.env['CLAUDE_BIN_DIR']).toBe('/custom/bin')
      expect(process.env['CLAUDE_CONFIG_DIR']).toBeUndefined()
    })

    it('should not set environment variables for whitespace-only strings', async () => {
      // Create mock GitHub service
      const mockGitHubService = {
        apiBaseUrl: 'https://api.github.com',
        getAuthenticatedUser: vi.fn().mockResolvedValue(null),
        getUserEmails: vi.fn().mockResolvedValue(null),
        validateToken: vi.fn().mockResolvedValue(false),
        getTokenScopes: vi.fn().mockResolvedValue(null),
      } as any

      // Set settings with whitespace-only strings
      const { SettingsService } = await import('../../../../services/settings.service.js')
      const settingsService = new SettingsService(mockGitHubService)
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '   ', // whitespace only
          configDir: '\t\n', // whitespace only
        },
      })

      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalled()

      // Environment variables should not be set for whitespace-only strings
      expect(process.env['CLAUDE_BIN_DIR']).toBeUndefined()
      expect(process.env['CLAUDE_CONFIG_DIR']).toBeUndefined()
    })

    it('should reset environment variables when settings are cleared', async () => {
      // Create mock GitHub service
      const mockGitHubService = {
        apiBaseUrl: 'https://api.github.com',
        getAuthenticatedUser: vi.fn().mockResolvedValue(null),
        getUserEmails: vi.fn().mockResolvedValue(null),
        validateToken: vi.fn().mockResolvedValue(false),
        getTokenScopes: vi.fn().mockResolvedValue(null),
      } as any

      // First set some settings
      const { SettingsService } = await import('../../../../services/settings.service.js')
      const settingsService = new SettingsService(mockGitHubService)
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      })

      // Run agent to set env vars
      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
      })

      expect(process.env['CLAUDE_BIN_DIR']).toBe('/custom/bin')
      expect(process.env['CLAUDE_CONFIG_DIR']).toBe('/custom/config')

      // Clear settings
      await settingsService.updateSettings({
        claude: {
          binDir: '',
          configDir: '',
        },
      })

      // Create a new agent to ensure it picks up updated settings
      const newAgent = new ClaudeCodeAgent('test-task-id-2', {
        model: 'claude-3-sonnet-20240320',
      })

      // Clear env vars to simulate fresh start
      Reflect.deleteProperty(process.env, 'CLAUDE_BIN_DIR')
      Reflect.deleteProperty(process.env, 'CLAUDE_CONFIG_DIR')

      // Run agent again
      await runAgent(newAgent, 'test message 2', {
        workspaceDir: '/test/workspace',
      })

      // Environment variables should not be set with empty settings
      expect(process.env['CLAUDE_BIN_DIR']).toBeUndefined()
      expect(process.env['CLAUDE_CONFIG_DIR']).toBeUndefined()

      // Verify settings are actually empty
      const claudeSettings = settingsService.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('')
      expect(claudeSettings.configDir).toBe('')
    })
  })

  describe('SDK options integration', () => {
    it('should pass correct options to Claude Code SDK', async () => {
      await runAgent(agent, 'test message', {
        workspaceDir: '/test/workspace',
        resume: 'test-session-id',
      })

      expect(vi.mocked(mockQuery)).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: 'test message',
          options: expect.objectContaining({
            allowedTools: ['WebFetch', 'WebSearch', 'Bash'],
            permissionMode: 'acceptEdits',
            cwd: '/test/workspace',
            model: 'claude-3-sonnet-20240320',
            resume: 'test-session-id',
            abortController: expect.any(AbortController),
          }),
        }),
      )
    })

    it('should use agent options cwd as fallback', async () => {
      const agentWithCwd = new ClaudeCodeAgent('test-task-id', {
        model: 'claude-3-sonnet-20240320',
        cwd: '/fallback/workspace',
      })

      await runAgent(agentWithCwd, 'test message', {})

      expect(vi.mocked(mockQuery)).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: 'test message',
          options: expect.objectContaining({
            cwd: '/fallback/workspace',
            abortController: expect.any(AbortController),
          }),
        }),
      )
    })

    it('should use process.cwd() as final fallback', async () => {
      await runAgent(agent, 'test message', {})

      expect(vi.mocked(mockQuery)).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: 'test message',
          options: expect.objectContaining({
            cwd: process.cwd(),
            abortController: expect.any(AbortController),
          }),
        }),
      )
    })
  })
})
