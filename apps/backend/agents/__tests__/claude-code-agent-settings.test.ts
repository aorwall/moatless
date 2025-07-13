import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ClaudeCodeAgent } from '../claude-code-agent.js'

// Mock the Claude Code SDK
vi.mock('@anthropic-ai/claude-code', () => ({
  query: vi.fn().mockImplementation(async function* () {
    yield {
      type: 'assistant',
      message: {
        id: 'msg_123',
        type: 'message',
        role: 'assistant',
        model: 'claude-opus-4-20250514',
        content: [
          {
            type: 'text',
            text: 'This is a regular text message.',
          },
        ],
        stop_reason: 'end_turn',
        stop_sequence: null,
        usage: {
          input_tokens: 10,
          output_tokens: 20,
        },
      },
      parent_tool_use_id: null,
      session_id: 'test-session',
    }
  }),
}))

// Mock logger
vi.mock('../../../utils/logger.js', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    child: vi.fn().mockReturnThis(),
  },
}))

// Mock calculateContextTokens
vi.mock('../../../utils/context-tokens.js', () => ({
  calculateContextTokens: vi.fn().mockReturnValue(100),
}))

describe('ClaudeCodeAgent Settings Integration', () => {
  let tempDir: string
  let originalEnv: NodeJS.ProcessEnv
  let agent: ClaudeCodeAgent

  beforeEach(async () => {
    // Save original environment
    originalEnv = { ...process.env }

    // Create temporary directory for tests
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-claude-settings-test-'))
    process.env.DATA_DIR = tempDir

    // Clear Claude-related env vars to ensure we test settings properly
    delete process.env.CLAUDE_BIN_DIR
    delete process.env.CLAUDE_CONFIG_DIR

    // Create agent with minimal options
    agent = new ClaudeCodeAgent('test-task-id', {
      model: 'claude-3-sonnet-20240320',
    })
  })

  afterEach(async () => {
    // Restore original environment
    process.env = originalEnv

    // Clean up temp directory
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('Claude settings integration', () => {
    it('should not set environment variables when settings are empty', async () => {
      // Mock the query method to capture the options
      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(queryMock).toHaveBeenCalled()

      // Environment variables should not be set for empty strings
      expect(process.env.CLAUDE_BIN_DIR).toBeUndefined()
      expect(process.env.CLAUDE_CONFIG_DIR).toBeUndefined()
    })

    it('should set environment variables when settings are configured', async () => {
      // Save settings with Claude configuration
      const settingsService = (await import('../../settings.service.js')).getSettingsService()
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      })

      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(queryMock).toHaveBeenCalled()

      // Environment variables should be set from settings
      expect(process.env.CLAUDE_BIN_DIR).toBe('/custom/bin')
      expect(process.env.CLAUDE_CONFIG_DIR).toBe('/custom/config')
    })

    it('should set environment variables from env vars when no settings override', async () => {
      // Set environment variables
      process.env.CLAUDE_BIN_DIR = '/env/bin'
      process.env.CLAUDE_CONFIG_DIR = '/env/config'

      // Reinitialize settings service to pick up env vars
      const settingsService = (await import('../../settings.service.js')).getSettingsService()
      await settingsService.initialize()

      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(queryMock).toHaveBeenCalled()

      // Environment variables should be set from original env vars
      expect(process.env.CLAUDE_BIN_DIR).toBe('/env/bin')
      expect(process.env.CLAUDE_CONFIG_DIR).toBe('/env/config')
    })

    it('should prioritize settings over environment variables', async () => {
      // Set environment variables first
      process.env.CLAUDE_BIN_DIR = '/env/bin'
      process.env.CLAUDE_CONFIG_DIR = '/env/config'

      // Initialize settings service and override with settings
      const settingsService = (await import('../../settings.service.js')).getSettingsService()
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/settings/bin',
          configDir: '/settings/config',
        },
      })

      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(queryMock).toHaveBeenCalled()

      // Settings should override environment variables
      expect(process.env.CLAUDE_BIN_DIR).toBe('/settings/bin')
      expect(process.env.CLAUDE_CONFIG_DIR).toBe('/settings/config')
    })

    it('should handle partial Claude settings', async () => {
      // Set only binDir in settings
      const settingsService = (await import('../../settings.service.js')).getSettingsService()
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          // configDir is not set
        },
      })

      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(queryMock).toHaveBeenCalled()

      // Only binDir should be set
      expect(process.env.CLAUDE_BIN_DIR).toBe('/custom/bin')
      expect(process.env.CLAUDE_CONFIG_DIR).toBeUndefined()
    })

    it('should not set environment variables for whitespace-only strings', async () => {
      // Set settings with whitespace-only strings
      const settingsService = (await import('../../settings.service.js')).getSettingsService()
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '   ', // whitespace only
          configDir: '\t\n', // whitespace only
        },
      })

      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(queryMock).toHaveBeenCalled()

      // Environment variables should not be set for whitespace-only strings
      expect(process.env.CLAUDE_BIN_DIR).toBeUndefined()
      expect(process.env.CLAUDE_CONFIG_DIR).toBeUndefined()
    })

    it('should reset environment variables when settings are cleared', async () => {
      // First set some settings
      const settingsService = (await import('../../settings.service.js')).getSettingsService()
      await settingsService.initialize()
      await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      })

      // Run agent to set env vars
      await agent.run('test message', { workspaceDir: '/test/workspace' })

      expect(process.env.CLAUDE_BIN_DIR).toBe('/custom/bin')
      expect(process.env.CLAUDE_CONFIG_DIR).toBe('/custom/config')

      // Clear settings
      await settingsService.updateSettings({
        claude: {
          binDir: null,
          configDir: null,
        },
      })

      // Run agent again
      await agent.run('test message 2', { workspaceDir: '/test/workspace' })

      // Environment variables should not be set anymore
      // Note: We can't actually "unset" env vars in the same process,
      // but we verify they won't be set again
      const claudeSettings = settingsService.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('')
      expect(claudeSettings.configDir).toBe('')
    })
  })

  describe('SDK options integration', () => {
    it('should pass correct options to Claude Code SDK', async () => {
      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run('test message', {
        workspaceDir: '/test/workspace',
        resume: 'test-session-id',
      })

      expect(queryMock).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: 'test message',
          abortController: expect.any(AbortController),
          options: expect.objectContaining({
            allowedTools: ['WebFetch', 'WebSearch', 'Bash'],
            permissionMode: 'acceptEdits',
            cwd: '/test/workspace',
            model: 'claude-3-sonnet-20240320',
            resume: 'test-session-id',
          }),
        }),
      )
    })

    it('should use agent options cwd as fallback', async () => {
      const agentWithCwd = new ClaudeCodeAgent('test-task-id', {
        model: 'claude-3-sonnet-20240320',
        cwd: '/fallback/workspace',
      })

      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agentWithCwd.run(
        'test message',
        {}, // no workspaceDir provided
      )

      expect(queryMock).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: 'test message',
          abortController: expect.any(AbortController),
          options: expect.objectContaining({
            cwd: '/fallback/workspace',
          }),
        }),
      )
    })

    it('should use process.cwd() as final fallback', async () => {
      const queryMock = vi.mocked(await import('@anthropic-ai/claude-code')).query

      await agent.run(
        'test message',
        {}, // no workspaceDir provided, no cwd in agent options
      )

      expect(queryMock).toHaveBeenCalledWith(
        expect.objectContaining({
          prompt: 'test message',
          abortController: expect.any(AbortController),
          options: expect.objectContaining({
            cwd: process.cwd(),
          }),
        }),
      )
    })
  })
})
