import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SettingsService } from '../settings.service'

// Mock logger
vi.mock('../../utils/logger.js', () => ({
  logger: {
    info: vi.fn(),
    error: vi.fn(),
    warn: vi.fn(),
  },
}))

// Mock git-utils
vi.mock('../../utils/git-utils.js', () => ({
  getGitUserName: vi.fn().mockResolvedValue('Test User'),
  getGitUserEmail: vi.fn().mockResolvedValue('test@example.com'),
}))

describe('SettingsService', () => {
  let tempDir: string
  let settingsService: SettingsService
  let originalEnv: NodeJS.ProcessEnv

  beforeEach(async () => {
    // Save original environment
    originalEnv = { ...process.env }

    // Create temporary directory for tests
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-settings-test-'))

    // Set test DATA_DIR
    process.env['DATA_DIR'] = tempDir

    // Create fresh settings service
    settingsService = new SettingsService()
    await settingsService.initialize()
  })

  afterEach(async () => {
    // Restore original environment
    process.env = originalEnv

    // Clean up temp directory
    await fs.rm(tempDir, { recursive: true, force: true })
  })

  describe('constructor and initialization', () => {
    it('should initialize with correct defaults', () => {
      const service = new SettingsService()

      // Defaults should include DATA_DIR env var
      expect(service.getDataDir()).toBe(tempDir)
      expect(service.getDefaultWorkspaceType()).toBe('local')

      const claudeSettings = service.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('')
      expect(claudeSettings.configDir).toBe('')
    })

    it('should use environment variables in defaults', () => {
      process.env['DATA_DIR'] = '/custom/data'
      process.env['DEFAULT_WORKSPACE_TYPE'] = 'docker'
      process.env['CLAUDE_BIN_DIR'] = '/custom/bin'
      process.env['CLAUDE_CONFIG_DIR'] = '/custom/config'

      const service = new SettingsService()

      expect(service.getDataDir()).toBe('/custom/data')
      expect(service.getDefaultWorkspaceType()).toBe('docker')

      const claudeSettings = service.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('/custom/bin')
      expect(claudeSettings.configDir).toBe('/custom/config')
    })

    it('should initialize without existing settings file', async () => {
      const service = new SettingsService()
      await service.initialize()

      const settings = await service.getSettings()
      expect(settings.settings).toEqual({})
    })
  })

  describe('getSettings', () => {
    it('should return empty settings and populated defaults', async () => {
      const result = await settingsService.getSettings()

      expect(result.settings).toEqual({})
      expect(result.defaults).toEqual({
        dataDir: tempDir,
        defaultWorkspaceType: 'local',
        binDir: '',
        configDir: '',
        oauthToken: '',
        claude: {
          binDir: '',
          configDir: '',
          oauthToken: '',
        },
      })
    })

    it('should return saved settings when they exist', async () => {
      const customDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'custom-data-'))

      try {
        // Update some settings first
        await settingsService.updateSettings({
          dataDir: customDataDir,
          defaultWorkspaceType: 'docker',
        })

        const result = await settingsService.getSettings()

        expect(result.settings).toEqual({
          dataDir: customDataDir,
          defaultWorkspaceType: 'docker',
        })
      } finally {
        await fs.rm(customDataDir, { recursive: true, force: true })
      }
    })
  })

  describe('updateSettings', () => {
    it('should update data directory', async () => {
      const newDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'new-data-'))

      try {
        const result = await settingsService.updateSettings({
          dataDir: newDataDir,
        })

        expect(result.settings.dataDir).toBe(newDataDir)
        expect(settingsService.getDataDir()).toBe(newDataDir)
      } finally {
        await fs.rm(newDataDir, { recursive: true, force: true })
      }
    })

    it('should update default workspace type', async () => {
      const result = await settingsService.updateSettings({
        defaultWorkspaceType: 'docker',
      })

      expect(result.settings.defaultWorkspaceType).toBe('docker')
      expect(settingsService.getDefaultWorkspaceType()).toBe('docker')
    })

    it('should update Claude settings', async () => {
      const result = await settingsService.updateSettings({
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      })

      expect(result.settings.claude).toEqual({
        binDir: '/custom/bin',
        configDir: '/custom/config',
      })

      const claudeSettings = settingsService.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('/custom/bin')
      expect(claudeSettings.configDir).toBe('/custom/config')
    })

    it('should handle null values (reset to defaults)', async () => {
      const customDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'custom-data-'))

      try {
        // First set some values
        await settingsService.updateSettings({
          dataDir: customDataDir,
          defaultWorkspaceType: 'docker',
          claude: {
            binDir: '/custom/bin',
            configDir: '/custom/config',
          },
        })

        // Then reset them with null values
        const result = await settingsService.updateSettings({
          dataDir: null,
          defaultWorkspaceType: null,
          claude: {
            binDir: null,
            configDir: null,
          },
        })

        expect(result.settings).toEqual({})
        expect(settingsService.getDataDir()).toBe(tempDir) // Should use default again
        expect(settingsService.getDefaultWorkspaceType()).toBe('local')
      } finally {
        await fs.rm(customDataDir, { recursive: true, force: true })
      }
    })

    it('should persist settings to file', async () => {
      const persistentDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'persistent-data-'))

      try {
        await settingsService.updateSettings({
          dataDir: persistentDataDir,
          defaultWorkspaceType: 'docker',
        })

        // Check that file was created in the new location
        const settingsPath = path.join(persistentDataDir, 'settings.json')
        const fileExists = await fs
          .access(settingsPath)
          .then(() => true)
          .catch(() => false)
        expect(fileExists).toBe(true)

        // Check file contents
        const fileContent = await fs.readFile(settingsPath, 'utf-8')
        const savedSettings = JSON.parse(fileContent)
        expect(savedSettings).toEqual({
          dataDir: persistentDataDir,
          defaultWorkspaceType: 'docker',
        })
      } finally {
        await fs.rm(persistentDataDir, { recursive: true, force: true })
      }
    })

    it('should move settings file when data directory changes', async () => {
      const newDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-new-data-'))

      try {
        // Create initial settings file
        await settingsService.updateSettings({
          defaultWorkspaceType: 'docker',
        })

        const oldSettingsPath = path.join(tempDir, 'settings.json')
        expect(
          await fs
            .access(oldSettingsPath)
            .then(() => true)
            .catch(() => false),
        ).toBe(true)

        // Change data directory
        await settingsService.updateSettings({
          dataDir: newDataDir,
        })

        // Old file should be gone, new file should exist
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

        // Content should be preserved
        const fileContent = await fs.readFile(newSettingsPath, 'utf-8')
        const savedSettings = JSON.parse(fileContent)
        expect(savedSettings.dataDir).toBe(newDataDir)
        expect(savedSettings.defaultWorkspaceType).toBe('docker')
      } finally {
        await fs.rm(newDataDir, { recursive: true, force: true })
      }
    })
  })

  describe('resetSettings', () => {
    it('should reset all settings to empty', async () => {
      const customDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'custom-data-'))

      try {
        // First set some settings
        await settingsService.updateSettings({
          dataDir: customDataDir,
          defaultWorkspaceType: 'docker',
          claude: {
            binDir: '/custom/bin',
            configDir: '/custom/config',
          },
        })

        // Reset settings
        const result = await settingsService.resetSettings()

        expect(result.settings).toEqual({})
        expect(settingsService.getDataDir()).toBe(tempDir) // Should use env var again
        expect(settingsService.getDefaultWorkspaceType()).toBe('local')

        const claudeSettings = settingsService.getClaudeSettings()
        expect(claudeSettings.binDir).toBe('')
        expect(claudeSettings.configDir).toBe('')
      } finally {
        await fs.rm(customDataDir, { recursive: true, force: true })
      }
    })

    it('should persist reset to file', async () => {
      const customDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'custom-data-'))

      try {
        // Set some settings first
        await settingsService.updateSettings({
          dataDir: customDataDir,
        })

        // Reset
        await settingsService.resetSettings()

        // Check file contents in current data directory (customDataDir)
        const settingsPath = path.join(customDataDir, 'settings.json')
        const fileContent = await fs.readFile(settingsPath, 'utf-8')
        const savedSettings = JSON.parse(fileContent)
        expect(savedSettings).toEqual({})
      } finally {
        await fs.rm(customDataDir, { recursive: true, force: true })
      }
    })
  })

  describe('getDataDir', () => {
    it('should prioritize settings over environment', async () => {
      const envDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'env-data-'))
      const settingsDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'settings-data-'))

      try {
        process.env['DATA_DIR'] = envDataDir

        const service = new SettingsService()
        await service.initialize()
        expect(service.getDataDir()).toBe(envDataDir)

        // Override with settings
        await service.updateSettings({ dataDir: settingsDataDir })
        expect(service.getDataDir()).toBe(settingsDataDir)
      } finally {
        await fs.rm(envDataDir, { recursive: true, force: true })
        await fs.rm(settingsDataDir, { recursive: true, force: true })
      }
    })

    it('should fall back to environment when settings is null', async () => {
      const envDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'env-data-'))
      const settingsDataDir = await fs.mkdtemp(path.join(os.tmpdir(), 'settings-data-'))

      try {
        process.env['DATA_DIR'] = envDataDir

        const service = new SettingsService()
        await service.initialize()
        await service.updateSettings({ dataDir: settingsDataDir })
        expect(service.getDataDir()).toBe(settingsDataDir)

        // Reset to null
        await service.updateSettings({ dataDir: null })
        expect(service.getDataDir()).toBe(envDataDir)
      } finally {
        await fs.rm(envDataDir, { recursive: true, force: true })
        await fs.rm(settingsDataDir, { recursive: true, force: true })
      }
    })
  })

  describe('getDefaultWorkspaceType', () => {
    it('should prioritize settings over environment', async () => {
      process.env['DEFAULT_WORKSPACE_TYPE'] = 'docker'

      const service = new SettingsService()
      await service.initialize()
      expect(service.getDefaultWorkspaceType()).toBe('docker')

      // Override with settings
      await service.updateSettings({ defaultWorkspaceType: 'local' })
      expect(service.getDefaultWorkspaceType()).toBe('local')
    })

    it('should handle invalid environment variable', () => {
      process.env['DEFAULT_WORKSPACE_TYPE'] = 'invalid'

      const service = new SettingsService()
      expect(service.getDefaultWorkspaceType()).toBe('local')
    })
  })

  describe('getClaudeSettings', () => {
    it('should prioritize settings over environment', async () => {
      process.env['CLAUDE_BIN_DIR'] = '/env/bin'
      process.env['CLAUDE_CONFIG_DIR'] = '/env/config'

      const service = new SettingsService()
      await service.initialize()

      let claudeSettings = service.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('/env/bin')
      expect(claudeSettings.configDir).toBe('/env/config')

      // Override with settings
      await service.updateSettings({
        claude: {
          binDir: '/settings/bin',
          configDir: '/settings/config',
        },
      })

      claudeSettings = service.getClaudeSettings()
      expect(claudeSettings.binDir).toBe('/settings/bin')
      expect(claudeSettings.configDir).toBe('/settings/config')
    })

    it('should return empty strings when no environment or settings', () => {
      Reflect.deleteProperty(process.env, 'CLAUDE_BIN_DIR')
      Reflect.deleteProperty(process.env, 'CLAUDE_CONFIG_DIR')

      const service = new SettingsService()
      const claudeSettings = service.getClaudeSettings()

      expect(claudeSettings.binDir).toBe('')
      expect(claudeSettings.configDir).toBe('')
    })
  })

  describe('error handling', () => {
    it('should handle file system errors gracefully', async () => {
      // Create service with invalid directory
      process.env['DATA_DIR'] = '/invalid/directory/that/does/not/exist'

      const service = new SettingsService()

      // Should not throw during initialization
      await expect(service.initialize()).resolves.not.toThrow()

      // Should still return defaults
      expect(service.getDataDir()).toBe('/invalid/directory/that/does/not/exist')
    })
  })
})
