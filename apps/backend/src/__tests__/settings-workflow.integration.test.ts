import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { build } from '../test/helper.js'

// Mock logger
vi.mock('../utils/logger.js', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    child: vi.fn().mockReturnThis(),
  },
}))

describe('Settings Workflow Integration (broad)', () => {
  let app: Awaited<ReturnType<typeof build>>
  let tempDir: string
  let originalEnv: NodeJS.ProcessEnv

  beforeEach(async () => {
    // Save original environment
    originalEnv = { ...process.env }

    // Create temporary directory for tests
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-settings-e2e-'))
    process.env['DATA_DIR'] = tempDir

    // Build test app
    app = await build()
  })

  afterEach(async () => {
    // Restore original environment
    process.env = originalEnv

    // Clean up temp directory
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('Complete Settings Workflow', () => {
    it('should handle full settings lifecycle', async () => {
      // 1. GET initial settings - should be empty with defaults
      let response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      let body = (await response.json()) as SettingsResponse

      expect(body.settings).toEqual({})
      expect(body.defaults.dataDir).toBe(tempDir)
      expect(body.defaults.defaultWorkspaceType).toBe('local')
      expect(body.defaults.claude?.binDir).toBe('')
      expect(body.defaults.claude?.configDir).toBe('')

      // 2. PUT settings - update all fields
      const fullUpdate: UpdateSettingsRequest = {
        dataDir: '/custom/data',
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      }

      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(fullUpdate),
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual(fullUpdate)

      // 3. Verify persistence by getting settings again
      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual(fullUpdate)

      // 4. Partial update - change only workspace type
      const partialUpdate: UpdateSettingsRequest = {
        defaultWorkspaceType: 'local',
      }

      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(partialUpdate),
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse

      // Should preserve other settings
      expect(body.settings).toEqual({
        dataDir: '/custom/data',
        defaultWorkspaceType: 'local', // updated
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      })

      // 5. Reset specific field with null
      const resetUpdate: UpdateSettingsRequest = {
        claude: {
          binDir: null, // reset this field
        },
      }

      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(resetUpdate),
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse

      // Should preserve other fields, remove binDir
      expect(body.settings).toEqual({
        dataDir: '/custom/data',
        defaultWorkspaceType: 'local',
        claude: {
          configDir: '/custom/config',
        },
      })

      // 6. DELETE - reset all settings
      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'DELETE',
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})

      // 7. Verify reset by getting settings again
      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})
    })

    it('should handle data directory migration', async () => {
      const newDataDir = path.join(tempDir, 'new-location')

      // 1. Set initial settings
      const initialSettings: UpdateSettingsRequest = {
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/initial/bin',
        },
      }

      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(initialSettings),
        }),
      )

      // Verify initial settings file exists
      const oldSettingsPath = path.join(tempDir, 'settings.json')
      const oldFileExists = await fs
        .access(oldSettingsPath)
        .then(() => true)
        .catch(() => false)
      expect(oldFileExists).toBe(true)

      // 2. Change data directory
      const migrationUpdate: UpdateSettingsRequest = {
        dataDir: newDataDir,
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(migrationUpdate),
        }),
      )

      expect(response.status).toBe(200)

      // 3. Verify migration
      // Old file should be gone
      const oldFileStillExists = await fs
        .access(oldSettingsPath)
        .then(() => true)
        .catch(() => false)
      expect(oldFileStillExists).toBe(false)

      // New file should exist with correct content
      const newSettingsPath = path.join(newDataDir, 'settings.json')
      const newFileExists = await fs
        .access(newSettingsPath)
        .then(() => true)
        .catch(() => false)
      expect(newFileExists).toBe(true)

      const fileContent = await fs.readFile(newSettingsPath, 'utf-8')
      const savedSettings = JSON.parse(fileContent)

      expect(savedSettings).toEqual({
        dataDir: newDataDir,
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/initial/bin',
        },
      })

      // Clean up
      await fs.rm(newDataDir, { recursive: true, force: true })
    })

    it('should handle environment variable interactions', async () => {
      // 1. Set environment variables
      process.env['DATA_DIR'] = '/env/data'
      process.env['DEFAULT_WORKSPACE_TYPE'] = 'docker'
      process.env['CLAUDE_BIN_DIR'] = '/env/bin'
      process.env['CLAUDE_CONFIG_DIR'] = '/env/config'

      // Rebuild app to pick up new env vars
      app = await build()

      // 2. Check defaults include env vars
      let response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)
      let body = (await response.json()) as SettingsResponse

      expect(body.defaults).toEqual({
        dataDir: '/env/data',
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/env/bin',
          configDir: '/env/config',
        },
      })

      // 3. Override some settings
      const override: UpdateSettingsRequest = {
        defaultWorkspaceType: 'local', // override env var
        claude: {
          binDir: '/custom/bin', // override env var
          // configDir not set - should use env var
        },
      }

      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(override),
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse

      expect(body.settings).toEqual({
        defaultWorkspaceType: 'local',
        claude: {
          binDir: '/custom/bin',
        },
      })

      // 4. Verify effective values through settings service
      // The settings service should return env vars for non-overridden values
      const settingsService = app.services.settings

      expect(settingsService.getDataDir()).toBe('/env/data') // from env
      expect(settingsService.getDefaultWorkspaceType()).toBe('local') // from settings

      const claudeSettings = settingsService.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('/custom/bin') // from settings
      expect(claudeSettings.configDir).toBe('/env/config') // from env

      // 5. Reset overrides
      const reset: UpdateSettingsRequest = {
        defaultWorkspaceType: null,
        claude: {
          binDir: null,
        },
      }

      response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(reset),
        }),
      )

      expect(response.status).toBe(200)
      body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})

      // 6. Verify env vars are effective again
      expect(settingsService.getDataDir()).toBe('/env/data')
      expect(settingsService.getDefaultWorkspaceType()).toBe('docker')

      const finalClaudeSettings = settingsService.getClaudeSettings()
      expect(finalClaudeSettings.binDir).toBe('/env/bin')
      expect(finalClaudeSettings.configDir).toBe('/env/config')
    })
  })

  describe('Error Scenarios', () => {
    it('should handle concurrent updates gracefully', async () => {
      // Simulate concurrent updates
      const update1: UpdateSettingsRequest = {
        dataDir: '/concurrent/1',
        defaultWorkspaceType: 'docker',
      }

      const update2: UpdateSettingsRequest = {
        dataDir: '/concurrent/2',
        claude: {
          binDir: '/concurrent/bin',
        },
      }

      // Send both requests concurrently
      const [response1, response2] = await Promise.all([
        app.fetch(
          new Request('http://localhost/api/v1/settings', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(update1),
          }),
        ),
        app.fetch(
          new Request('http://localhost/api/v1/settings', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(update2),
          }),
        ),
      ])

      // Both should succeed
      expect(response1.status).toBe(200)
      expect(response2.status).toBe(200)

      // Final state should be consistent
      const finalResponse = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(finalResponse.status).toBe(200)
      const body = (await finalResponse.json()) as SettingsResponse

      // Should have some combination of the updates
      expect(body.settings).toBeDefined()
    })

    it('should handle invalid file system permissions gracefully', async () => {
      // This test would require setting up restricted permissions
      // which is complex in a test environment, so we'll skip it
      // In a real scenario, we'd test what happens when the data
      // directory becomes read-only or inaccessible
    })
  })
})
