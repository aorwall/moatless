import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { SettingsResponse, UpdateSettingsRequest } from '@moatless/shared'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { resetSettingsServiceInstance } from '../../services/settings.service.js'
import { build } from '../../test/helper.js'

// Mock logger
vi.mock('../../utils/logger.js', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    child: vi.fn().mockReturnThis(),
  },
  createLogger: vi.fn().mockReturnValue({
    info: vi.fn(),
    error: vi.fn(),
    child: vi.fn().mockReturnThis(),
  }),
}))

describe('Settings Routes Integration', () => {
  let app: Awaited<ReturnType<typeof build>>
  let tempDir: string
  let originalEnv: NodeJS.ProcessEnv

  beforeEach(async () => {
    // Save original environment
    originalEnv = { ...process.env }

    // Reset settings service singleton to ensure clean state
    resetSettingsServiceInstance()

    // Create temporary directory for tests
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-settings-integration-'))
    process.env.DATA_DIR = tempDir

    // Build test app
    app = await build()
  })

  afterEach(async () => {
    // Restore original environment
    process.env = originalEnv

    // Reset settings service singleton
    resetSettingsServiceInstance()

    // Clean up temp directory
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('GET /api/v1/settings', () => {
    it('should return default settings when no settings file exists', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})
      expect(body.defaults).toEqual({
        dataDir: tempDir,
        defaultWorkspaceType: 'local',
        claude: {
          binDir: '',
          configDir: '',
        },
      })
    })

    it('should return saved settings when they exist', async () => {
      // First save some settings
      const updatePayload: UpdateSettingsRequest = {
        dataDir: '/custom/data',
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      }

      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(updatePayload),
        }),
      )

      // Then get settings
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual(updatePayload)
    })

    it('should include environment variables in defaults', async () => {
      // Reset settings service before changing environment
      resetSettingsServiceInstance()

      // Set environment variables to valid paths
      const envDataDir = path.join(tempDir, 'env-data')
      process.env.DATA_DIR = envDataDir
      process.env.DEFAULT_WORKSPACE_TYPE = 'docker'
      process.env.CLAUDE_BIN_DIR = '/env/bin'
      process.env.CLAUDE_CONFIG_DIR = '/env/config'

      // Rebuild app with new env vars
      app = await build()

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'GET',
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.defaults).toEqual({
        dataDir: envDataDir,
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/env/bin',
          configDir: '/env/config',
        },
      })
    })
  })

  describe('PUT /api/v1/settings', () => {
    it('should update and persist settings', async () => {
      // Use a valid directory path within our temp directory
      const newDataDir = path.join(tempDir, 'new-data-dir')

      const updatePayload: UpdateSettingsRequest = {
        dataDir: newDataDir,
        defaultWorkspaceType: 'docker',
        claude: {
          binDir: '/new/bin',
          configDir: '/new/config',
        },
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(updatePayload),
        }),
      )

      if (response.status !== 200) {
        const errorBody = await response.text()
        console.error('PUT /settings failed:', errorBody)
      }

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual(updatePayload)

      // Verify persistence by checking file in new data directory
      const settingsPath = path.join(newDataDir, 'settings.json')
      const fileContent = await fs.readFile(settingsPath, 'utf-8')
      const savedSettings = JSON.parse(fileContent)
      expect(savedSettings).toEqual(updatePayload)
    })

    it('should handle partial updates', async () => {
      // First set some initial settings with valid path
      const initialDataDir = path.join(tempDir, 'initial-data')
      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            dataDir: initialDataDir,
            defaultWorkspaceType: 'docker',
          }),
        }),
      )

      // Then update only one field
      const partialUpdate: UpdateSettingsRequest = {
        defaultWorkspaceType: 'local',
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(partialUpdate),
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({
        dataDir: initialDataDir,
        defaultWorkspaceType: 'local',
      })
    })

    it('should handle null values to reset to defaults', async () => {
      // Set some initial values
      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            dataDir: '/custom/data',
            defaultWorkspaceType: 'docker',
            claude: {
              binDir: '/custom/bin',
            },
          }),
        }),
      )

      // Reset with null values
      const resetUpdate: UpdateSettingsRequest = {
        dataDir: null,
        defaultWorkspaceType: null,
        claude: {
          binDir: null,
        },
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(resetUpdate),
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})
    })

    it('should validate request body schema', async () => {
      const invalidPayload = {
        dataDir: 123, // Should be string
        defaultWorkspaceType: 'invalid', // Should be 'docker' or 'local'
        claude: {
          binDir: true, // Should be string
        },
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(invalidPayload),
        }),
      )

      expect(response.status).toBe(400)

      const body = (await response.json()) as { error?: string }
      expect(body.error).toBeDefined()
    })

    it('should handle empty request body', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({}),
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})
    })

    it('should handle Claude settings partial updates', async () => {
      // Set initial Claude settings
      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            claude: {
              binDir: '/initial/bin',
              configDir: '/initial/config',
            },
          }),
        }),
      )

      // Update only binDir
      const partialUpdate: UpdateSettingsRequest = {
        claude: {
          binDir: '/updated/bin',
        },
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(partialUpdate),
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({
        claude: {
          binDir: '/updated/bin',
          configDir: '/initial/config',
        },
      })
    })
  })

  describe('DELETE /api/v1/settings', () => {
    it('should reset all settings to empty', async () => {
      // First set some settings, but keep data dir the same
      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            defaultWorkspaceType: 'docker',
            claude: {
              binDir: '/custom/bin',
              configDir: '/custom/config',
            },
          }),
        }),
      )

      // Reset settings
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'DELETE',
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})

      // Verify persistence
      const settingsPath = path.join(tempDir, 'settings.json')
      const fileContent = await fs.readFile(settingsPath, 'utf-8')
      const savedSettings = JSON.parse(fileContent)
      expect(savedSettings).toEqual({})
    })

    it('should work when no settings exist', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'DELETE',
        }),
      )

      expect(response.status).toBe(200)

      const body = (await response.json()) as SettingsResponse
      expect(body.settings).toEqual({})
    })
  })

  describe('error handling', () => {
    it('should handle malformed JSON', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: 'invalid json',
        }),
      )

      expect(response.status).toBe(400)
    })

    it('should handle missing content-type header', async () => {
      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          body: JSON.stringify({ dataDir: path.join(tempDir, 'test-dir') }),
        }),
      )

      // The zod validator doesn't enforce content-type, so it accepts the request
      expect(response.status).toBe(200)
    })
  })

  describe('settings file lifecycle', () => {
    it('should create settings directory if it does not exist', async () => {
      // Use a data dir that doesn't exist yet
      const nonExistentDir = path.join(tempDir, 'nested', 'directory')

      const updatePayload: UpdateSettingsRequest = {
        dataDir: nonExistentDir,
      }

      const response = await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(updatePayload),
        }),
      )

      expect(response.status).toBe(200)

      // Verify directory was created
      const dirExists = await fs
        .access(nonExistentDir)
        .then(() => true)
        .catch(() => false)
      expect(dirExists).toBe(true)

      // Verify settings file exists in the new data directory
      const settingsPath = path.join(nonExistentDir, 'settings.json')
      const fileExists = await fs
        .access(settingsPath)
        .then(() => true)
        .catch(() => false)
      expect(fileExists).toBe(true)
    })

    it('should move settings file when data directory changes', async () => {
      const newDataDir = path.join(tempDir, 'new-data-dir')

      // Create initial settings
      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            defaultWorkspaceType: 'docker',
          }),
        }),
      )

      const oldSettingsPath = path.join(tempDir, 'settings.json')
      expect(
        await fs
          .access(oldSettingsPath)
          .then(() => true)
          .catch(() => false),
      ).toBe(true)

      // Change data directory
      await app.fetch(
        new Request('http://localhost/api/v1/settings', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            dataDir: newDataDir,
          }),
        }),
      )

      // Old settings file should be moved to new location
      expect(
        await fs
          .access(oldSettingsPath)
          .then(() => true)
          .catch(() => false),
      ).toBe(false)

      const newSettingsPath = path.join(newDataDir, 'settings.json')
      expect(
        await fs
          .access(newSettingsPath)
          .then(() => true)
          .catch(() => false),
      ).toBe(true)

      // Settings should be preserved
      const fileContent = await fs.readFile(newSettingsPath, 'utf-8')
      const savedSettings = JSON.parse(fileContent)
      expect(savedSettings.dataDir).toBe(newDataDir)
      expect(savedSettings.defaultWorkspaceType).toBe('docker')

      // Clean up
      await fs.rm(newDataDir, { recursive: true, force: true })
    })
  })
})
