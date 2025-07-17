import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type {
  ApplicationSettings,
  ClaudeSettings,
  SettingsResponse,
  SystemStatusCheck,
  UpdateSettingsRequest,
} from '@moatless/types'
import { checkSystemStatus } from '../agents/claude-code/claude-utils.js'
import { logger } from '../utils/logger.js'

export class SettingsService {
  private settings: ApplicationSettings | null = null
private settingsPath: string | null = null;
  private defaults: SettingsResponse['defaults']

  constructor(private settingsPath: string | null = null) {
    // Initialize defaults
    this.defaults = {
      dataDir: process.env.DATA_DIR || path.join(os.homedir(), '.moatless'),
      defaultWorkspaceType: (process.env.DEFAULT_WORKSPACE_TYPE === 'docker'
        ? 'docker'
        : 'local') as 'docker' | 'local',
      claude: {
        binDir: process.env.CLAUDE_BIN_DIR || '',
        configDir: process.env.CLAUDE_CONFIG_DIR || '',
      },
    }
  }

  async initialize(): Promise<void> {
    try {
      // Load settings from file if it exists
      await this.loadSettings()
    } catch (error) {
      logger.error('Failed to initialize settings service', error as Error)
      // Continue with defaults if loading fails
    }
  }

  private async loadSettings(): Promise<void> {
    const dataDir = this.getDataDir()
    this.settingsPath = path.join(dataDir, 'settings.json')

    try {
      // Ensure data directory exists
      await fs.mkdir(dataDir, { recursive: true })

      // Try to read existing settings
      const settingsData = await fs.readFile(this.settingsPath, 'utf-8')
      this.settings = JSON.parse(settingsData)
      logger.info('Loaded settings from file', { settingsPath: this.settingsPath })
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        logger.info('No settings file found, using defaults', { settingsPath: this.settingsPath })
        this.settings = {}
      } else {
        logger.error('Failed to load settings', error as Error, { settingsPath: this.settingsPath })
        throw error
      }
    }
  }

  private async saveSettings(): Promise<void> {
    if (!this.settingsPath) {
      throw new Error('Settings path not initialized')
    }

    const dataDir = path.dirname(this.settingsPath)
    await fs.mkdir(dataDir, { recursive: true })

    await fs.writeFile(this.settingsPath, JSON.stringify(this.settings || {}, null, 2))
    logger.info('Saved settings to file', { settingsPath: this.settingsPath })
  }

  getDataDir(): string {
    // First check if settings have a custom data dir
    if (this.settings?.dataDir) {
      return this.settings.dataDir
    }

    // Use defaults (which include env var if set, otherwise fallback)
    return this.defaults.dataDir
  }

  getDefaultWorkspaceType(): 'docker' | 'local' {
    // First check settings
    if (this.settings?.defaultWorkspaceType) {
      return this.settings.defaultWorkspaceType
    }

    // Use defaults (which include env var if set, otherwise fallback)
    return this.defaults.defaultWorkspaceType
  }

  getClaudeSettings(): Required<ClaudeSettings> {
    return {
      binDir: this.settings?.claude?.binDir || this.defaults.claude.binDir,
      configDir: this.settings?.claude?.configDir || this.defaults.claude.configDir,
    }
  }

  async getSettings(): Promise<SettingsResponse> {
    return {
      settings: this.settings || {},
      defaults: this.defaults,
    }
  }

  async updateSettings(update: UpdateSettingsRequest): Promise<SettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    // Update data dir
    if (update.dataDir !== undefined) {
      if (update.dataDir === null) {
        delete this.settings.dataDir
      } else {
        this.settings.dataDir = update.dataDir
      }
    }

    // Update default workspace type
    if (update.defaultWorkspaceType !== undefined) {
      if (update.defaultWorkspaceType === null) {
        delete this.settings.defaultWorkspaceType
      } else {
        this.settings.defaultWorkspaceType = update.defaultWorkspaceType
      }
    }

    // Update Claude settings
    if (update.claude) {
      if (!this.settings.claude) {
        this.settings.claude = {}
      }

      if (update.claude.binDir !== undefined) {
        if (update.claude.binDir === null) {
          delete this.settings.claude.binDir
        } else {
          this.settings.claude.binDir = update.claude.binDir
        }
      }

      if (update.claude.configDir !== undefined) {
        if (update.claude.configDir === null) {
          delete this.settings.claude.configDir
        } else {
          this.settings.claude.configDir = update.claude.configDir
        }
      }

      // Remove claude object if empty
      if (Object.keys(this.settings.claude).length === 0) {
        delete this.settings.claude
      }
    }

    // Remove settings object if empty
    if (Object.keys(this.settings).length === 0) {
      this.settings = {}
    }

    // Update settings path based on new data dir
    const newDataDir = this.getDataDir()
    const newSettingsPath = path.join(newDataDir, 'settings.json')

    // If settings path changed, move the file
    if (this.settingsPath && this.settingsPath !== newSettingsPath) {
      try {
        // Ensure new directory exists
        await fs.mkdir(newDataDir, { recursive: true })

        // Copy settings to new location
        await fs.copyFile(this.settingsPath, newSettingsPath)

        // Delete old file
        await fs.unlink(this.settingsPath)

        logger.info('Moved settings file to new location', {
          oldPath: this.settingsPath,
          newPath: newSettingsPath,
        })
      } catch (error) {
        logger.error('Failed to move settings file', error as Error, {
          oldPath: this.settingsPath,
          newPath: newSettingsPath,
        })
      }
    }

    this.settingsPath = newSettingsPath

    // Save updated settings
    await this.saveSettings()

    return this.getSettings()
  }

  async resetSettings(): Promise<SettingsResponse> {
    this.settings = {}

    // Recalculate settings path based on default data dir
    const defaultDataDir = this.getDataDir()
    const newSettingsPath = path.join(defaultDataDir, 'settings.json')

    // If path changed, move the file
    if (this.settingsPath && this.settingsPath !== newSettingsPath) {
      try {
        // Ensure new directory exists
        await fs.mkdir(defaultDataDir, { recursive: true })

        // Move settings file to new location
        await fs.copyFile(this.settingsPath, newSettingsPath)
        await fs.unlink(this.settingsPath)

        logger.info('Moved settings file during reset', {
          oldPath: this.settingsPath,
          newPath: newSettingsPath,
        })
      } catch (error) {
        logger.error('Failed to move settings file during reset', error as Error, {
          oldPath: this.settingsPath,
          newPath: newSettingsPath,
        })
      }
    }

    this.settingsPath = newSettingsPath
    await this.saveSettings()
    return this.getSettings()
  }

  async getSystemStatus(): Promise<SystemStatusCheck> {
    return checkSystemStatus()
  }
}

// Singleton instance
let settingsServiceInstance: SettingsService | null = null

export function getSettingsService(): SettingsService {
  if (!settingsServiceInstance) {
    settingsServiceInstance = new SettingsService()
  }
  return settingsServiceInstance
}

// For testing: reset the singleton instance
export function resetSettingsServiceInstance(): void {
  settingsServiceInstance = null
}

export async function initializeSettingsService(): Promise<SettingsService> {
  const service = getSettingsService()
  await service.initialize()
  return service
}

// Factory function for dependency injection
export async function createSettingsService(settingsPath?: string): Promise<SettingsService> {
  const service = new SettingsService(settingsPath)
  await service.initialize()
  return service
}
