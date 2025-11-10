import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type {
  ApplicationSettings,
  ClaudeSettings,
  ClaudeSettingsResponse,
  GeneralSettingsResponse,
  McpSettings,
  McpSettingsResponse,
  PublishSettings,
  PublishSettingsResponse,
  SettingsResponse,
  SystemStatusCheck,
  UpdateClaudeSettingsRequest,
  UpdateGeneralSettingsRequest,
  UpdateMcpSettingsRequest,
  UpdatePublishSettingsRequest,
  UpdateSettingsRequest,
} from '@moatless/types'
import { injectable } from 'inversify'
import { checkSystemStatus } from '../features/agents/implementations/claude-code/claude-utils'
import { logger } from '../utils/logger'

/**
 * SettingsService - Application-wide settings management
 *
 * This service manages persistent application settings stored in settings.json.
 * It remains in the services directory as a cross-feature infrastructure service
 * because it is used by multiple features:
 * - Agents feature (Claude settings, MCP settings)
 * - Workspace feature (execution mode, data directory)
 * - Repositories feature (repositories directory, worktree settings)
 * - Settings routes (dedicated settings API)
 *
 * Moving this service to a specific feature would create circular dependencies
 * since it needs to be accessible to all features while maintaining a singleton
 * state for application-wide configuration.
 *
 * Special initialization: This service requires bootstrap before the DI container
 * is fully initialized (see container.ts lines 40-43).
 */
@injectable()
export class SettingsService {
  private settings: ApplicationSettings | null = null
  private defaults: {
    dataDir: string
    repositoriesDir: string
    defaultWorkspaceType: 'docker' | 'local'
    claude: {
      binDir: string
      configDir: string
      oauthToken: string
    }
    mcp: McpSettings
    publish: {
      netlifyAccessToken: string
    }
  }

  constructor(private settingsPath: string | null = null) {
    // Initialize defaults with placeholder values - will be updated in initialize()
    this.defaults = {
      dataDir: process.env['DATA_DIR'] || path.join(os.homedir(), '.moatless'),
      repositoriesDir:
        process.env['REPO_DIR'] ||
        path.join(process.env['DATA_DIR'] || path.join(os.homedir(), '.moatless'), 'repos'),
      defaultWorkspaceType: (process.env['DEFAULT_WORKSPACE_TYPE'] === 'docker'
        ? 'docker'
        : 'local') as 'docker' | 'local',
      claude: {
        binDir: process.env['CLAUDE_BIN_DIR'] || '',
        configDir: process.env['CLAUDE_CONFIG_DIR'] || '',
        oauthToken: process.env['CLAUDE_CODE_OAUTH_TOKEN'] || '',
      },
      mcp: {
        mcpServers: {},
      },
      publish: {
        netlifyAccessToken: process.env['NETLIFY_ACCESS_TOKEN'] || '',
      },
    }
  }

  async initialize(): Promise<void> {
    try {
      // Try to detect Claude paths if not set
      if (!(this.defaults.claude.binDir && this.defaults.claude.configDir)) {
        await this.detectClaudePaths()
      }

      // Load settings from file if it exists
      await this.loadSettings()
    } catch (error) {
      logger.error('Failed to initialize settings service', error as Error)
      // Continue with defaults if loading fails
    }
  }

  private async detectClaudePaths(): Promise<void> {
    try {
      // Import ClaudeService dynamically to avoid circular dependency
      const { ClaudeService } = await import('../features/claude/index.js')

      // Create a temporary instance just for path detection
      const tempClaudeService = new ClaudeService(this)

      // Detect bin directory if not set
      if (!this.defaults.claude.binDir) {
        const detectedBinDir = tempClaudeService.findClaudeBinDir()
        if (detectedBinDir) {
          // Validate that claude binary actually works
          const validation = await tempClaudeService.validateClaudePaths(detectedBinDir, '')
          if (validation.binDirValid) {
            this.defaults.claude.binDir = detectedBinDir
            logger.info('Detected and validated Claude bin directory', {
              binDir: detectedBinDir,
              version: validation.claudeVersion,
            })
          } else {
            logger.warn('Detected Claude bin directory but validation failed', {
              binDir: detectedBinDir,
              error: validation.error,
            })
          }
        }
      }

      // Detect config directory if not set
      if (!this.defaults.claude.configDir) {
        const detectedConfigDir = tempClaudeService.findClaudeConfigDir()
        if (detectedConfigDir) {
          this.defaults.claude.configDir = detectedConfigDir
          logger.info('Detected Claude config directory', {
            configDir: detectedConfigDir,
          })
        }
      }
    } catch (error) {
      logger.debug('Could not detect Claude paths', { error })
      // Continue without detected paths
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
      logger.info('Loaded settings from file', {
        settingsPath: this.settingsPath,
      })
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        logger.info('No settings file found, using defaults', {
          settingsPath: this.settingsPath,
        })
        this.settings = {}
      } else {
        logger.error('Failed to load settings', error as Error, {
          settingsPath: this.settingsPath,
        })
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

  getRepositoriesDir(): string {
    // First check if settings have a custom repositories dir
    if (this.settings?.repositoriesDir) {
      return this.settings.repositoriesDir
    }

    // Use defaults (which include env var if set, otherwise fallback)
    return this.defaults.repositoriesDir
  }

  getDefaultWorkspaceType(): 'docker' | 'local' {
    // First check settings
    if (this.settings?.defaultWorkspaceType) {
      return this.settings.defaultWorkspaceType
    }

    // Use defaults (which include env var if set, otherwise fallback)
    return this.defaults.defaultWorkspaceType
  }

  /**
   * Get workspace execution mode from environment variable
   * This is a system-wide setting that determines which environment driver to use
   * @returns 'local' or 'docker' based on WORKSPACE_EXECUTION_MODE env var (defaults to 'local')
   */
  getWorkspaceExecutionMode(): 'local' | 'docker' {
    const mode = process.env['WORKSPACE_EXECUTION_MODE']
    return mode === 'docker' ? 'docker' : 'local'
  }

  getClaudeSettings(): Required<ClaudeSettings> {
    return {
      binDir: this.settings?.claude?.binDir || this.settings?.binDir || this.defaults.claude.binDir,
      configDir:
        this.settings?.claude?.configDir ||
        this.settings?.configDir ||
        this.defaults.claude.configDir,
      oauthToken:
        this.settings?.claude?.oauthToken ||
        this.settings?.oauthToken ||
        this.defaults.claude.oauthToken,
    }
  }

  getGeneralSettings(): GeneralSettingsResponse {
    return {
      settings: {
        dataDir: this.settings?.dataDir,
        repositoriesDir: this.settings?.repositoriesDir,
        defaultWorkspaceType: this.settings?.defaultWorkspaceType,
      },
      defaults: {
        dataDir: this.defaults.dataDir,
        repositoriesDir: this.defaults.repositoriesDir,
        defaultWorkspaceType: this.defaults.defaultWorkspaceType,
      },
    }
  }

  async getClaudeSettingsResponse(): Promise<ClaudeSettingsResponse> {
    // Get configured values or defaults (which may have been detected)
    let binDir = this.settings?.binDir || this.defaults.claude.binDir
    let configDir = this.settings?.configDir || this.defaults.claude.configDir
    const oauthToken = this.settings?.oauthToken || this.defaults.claude.oauthToken

    // If paths are still empty, try to detect them now
    if (!(binDir && configDir)) {
      try {
        const { ClaudeService } = await import('../features/claude/index.js')
        const tempClaudeService = new ClaudeService(this)

        if (!binDir) {
          const detectedBinDir = tempClaudeService.findClaudeBinDir()
          if (detectedBinDir) {
            binDir = detectedBinDir
          }
        }

        if (!configDir) {
          const detectedConfigDir = tempClaudeService.findClaudeConfigDir()
          if (detectedConfigDir) {
            configDir = detectedConfigDir
          }
        }
      } catch (error) {
        logger.debug('Could not detect Claude paths in getClaudeSettingsResponse', { error })
      }
    }

    // Check if paths are valid
    const path = await import('node:path')
    const { existsSync } = await import('node:fs')

    // Check if binDir exists and contains claude executable
    let binDirValid = false
    if (binDir) {
      const claudeBinPath = path.join(binDir, 'claude')
      binDirValid = existsSync(claudeBinPath)
    }

    // Check if configDir exists
    const configDirValid = configDir ? existsSync(configDir) : false

    // Check if values were overridden by user
    const binDirOverridden = Boolean(this.settings?.binDir)
    const configDirOverridden = Boolean(this.settings?.configDir)

    return {
      binDir,
      configDir,
      oauthToken,
      binDirValid,
      configDirValid,
      binDirOverridden,
      configDirOverridden,
    }
  }

  getPublishSettings(): Required<PublishSettings> {
    return {
      netlifyAccessToken:
        this.settings?.publish?.netlifyAccessToken || this.defaults.publish.netlifyAccessToken,
    }
  }

  getPublishSettingsResponse(): PublishSettingsResponse {
    return {
      settings: {
        netlifyAccessToken: this.settings?.publish?.netlifyAccessToken,
      },
      defaults: this.defaults.publish,
    }
  }

  async getMcpSettings(): Promise<Required<McpSettings>> {
    // Reload settings from disk to ensure we have the latest MCP configuration
    await this.loadSettings()
    return {
      mcpServers: this.settings?.mcp?.mcpServers || this.defaults.mcp.mcpServers,
    }
  }

  getMcpSettingsResponse(): McpSettingsResponse {
    return {
      settings: {
        mcpServers: this.settings?.mcp?.mcpServers || {},
      },
      defaults: {
        mcpServers: this.defaults.mcp.mcpServers,
      },
    }
  }

  async updateMcpSettingsSection(update: UpdateMcpSettingsRequest): Promise<McpSettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    // Update MCP settings
    if (!this.settings.mcp) {
      this.settings.mcp = { mcpServers: {} }
    }

    if (update.mcpServers !== undefined) {
      if (update.mcpServers === null) {
        this.settings.mcp.mcpServers = {}
      } else {
        this.settings.mcp.mcpServers = update.mcpServers
      }
    }

    // Remove mcp object if empty
    if (Object.keys(this.settings.mcp.mcpServers).length === 0) {
      Reflect.deleteProperty(this.settings, 'mcp')
    }

    // Save updated settings
    await this.saveSettings()

    return this.getMcpSettingsResponse()
  }

  async updateGeneralSettings(
    update: UpdateGeneralSettingsRequest,
  ): Promise<GeneralSettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    // Update data dir
    if (update.dataDir !== undefined) {
      if (update.dataDir === null) {
        Reflect.deleteProperty(this.settings, 'dataDir')
      } else {
        this.settings.dataDir = update.dataDir
      }
    }

    // Update repositories dir
    if (update.repositoriesDir !== undefined) {
      if (update.repositoriesDir === null) {
        Reflect.deleteProperty(this.settings, 'repositoriesDir')
      } else {
        this.settings.repositoriesDir = update.repositoriesDir
      }
    }

    // Update default workspace type
    if (update.defaultWorkspaceType !== undefined) {
      if (update.defaultWorkspaceType === null) {
        Reflect.deleteProperty(this.settings, 'defaultWorkspaceType')
      } else {
        this.settings.defaultWorkspaceType = update.defaultWorkspaceType
      }
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

    return this.getGeneralSettings()
  }

  async updateClaudeSettingsSection(
    update: UpdateClaudeSettingsRequest,
  ): Promise<ClaudeSettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    // Update Claude settings at root level
    if (update.binDir !== undefined) {
      if (update.binDir === null) {
        Reflect.deleteProperty(this.settings, 'binDir')
      } else {
        this.settings.binDir = update.binDir
      }
    }

    if (update.configDir !== undefined) {
      if (update.configDir === null) {
        Reflect.deleteProperty(this.settings, 'configDir')
      } else {
        this.settings.configDir = update.configDir
      }
    }

    if (update.oauthToken !== undefined) {
      if (update.oauthToken === null) {
        Reflect.deleteProperty(this.settings, 'oauthToken')
      } else {
        this.settings.oauthToken = update.oauthToken
      }
    }

    // Save updated settings
    await this.saveSettings()

    return this.getClaudeSettingsResponse()
  }

  async updatePublishSettingsSection(
    update: UpdatePublishSettingsRequest,
  ): Promise<PublishSettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    // Update Publish settings
    if (!this.settings.publish) {
      this.settings.publish = {}
    }

    if (update.netlifyAccessToken !== undefined) {
      if (update.netlifyAccessToken === null) {
        if (this.settings.publish) {
          Reflect.deleteProperty(this.settings.publish, 'netlifyAccessToken')
        }
      } else {
        this.settings.publish.netlifyAccessToken = update.netlifyAccessToken
      }
    }

    // Remove publish object if empty
    if (Object.keys(this.settings.publish).length === 0) {
      Reflect.deleteProperty(this.settings, 'publish')
    }

    // Save updated settings
    await this.saveSettings()

    return this.getPublishSettingsResponse()
  }

  async getSystemStatus(): Promise<SystemStatusCheck> {
    return await checkSystemStatus()
  }

  async getSettings(): Promise<SettingsResponse> {
    const [, status] = await Promise.all([this.getGeneralSettings(), this.getSystemStatus()])

    // Build full settings and defaults objects for the response
    const fullSettings: ApplicationSettings = {
      dataDir: this.settings?.dataDir,
      repositoriesDir: this.settings?.repositoriesDir,
      defaultWorkspaceType: this.settings?.defaultWorkspaceType,
      binDir: this.settings?.binDir,
      configDir: this.settings?.configDir,
      oauthToken: this.settings?.oauthToken,
      claude: this.settings?.claude
        ? {
            binDir: this.settings.claude.binDir,
            configDir: this.settings.claude.configDir,
            oauthToken: this.settings.claude.oauthToken,
          }
        : undefined,
      userGroups: this.settings?.userGroups,
    }

    const fullDefaults: ApplicationSettings = {
      dataDir: this.defaults.dataDir,
      repositoriesDir: this.defaults.repositoriesDir,
      defaultWorkspaceType: this.defaults.defaultWorkspaceType,
      binDir: this.defaults.claude.binDir,
      configDir: this.defaults.claude.configDir,
      oauthToken: this.defaults.claude.oauthToken,
      claude: {
        binDir: this.defaults.claude.binDir,
        configDir: this.defaults.claude.configDir,
        oauthToken: this.defaults.claude.oauthToken,
      },
    }

    return {
      settings: fullSettings,
      status,
      defaults: fullDefaults,
    }
  }

  async updateSettings(update: UpdateSettingsRequest): Promise<SettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    this.updateGeneralSettingsFields(update)
    this.updateLegacyClaudeFields(update)
    this.updateClaudeSettings(update)
    this.updatePublishSettings(update)
    await this.handleDataDirChange(update)

    await this.saveSettings()
    return this.getSettings()
  }

  private updateGeneralSettingsFields(update: UpdateSettingsRequest): void {
    this.updateField('dataDir', update.dataDir)
    this.updateField('repositoriesDir', update.repositoriesDir)
    this.updateField('defaultWorkspaceType', update.defaultWorkspaceType)
  }

  private updateLegacyClaudeFields(update: UpdateSettingsRequest): void {
    this.updateField('binDir', update.binDir)
    this.updateField('configDir', update.configDir)
    this.updateField('oauthToken', update.oauthToken)
  }

  private updateClaudeSettings(update: UpdateSettingsRequest): void {
    if (update.claude === undefined || !this.settings) {
      return
    }

    if (update.claude === null) {
      Reflect.deleteProperty(this.settings, 'claude')
      return
    }

    if (!this.settings.claude) {
      this.settings.claude = {}
    }

    this.updateNestedField(this.settings.claude, 'binDir', update.claude.binDir)
    this.updateNestedField(this.settings.claude, 'configDir', update.claude.configDir)
    this.updateNestedField(this.settings.claude, 'oauthToken', update.claude.oauthToken)

    this.removeIfEmpty('claude')
  }

  private updatePublishSettings(update: UpdateSettingsRequest): void {
    if (update.publish === undefined || !this.settings) {
      return
    }

    if (update.publish === null) {
      Reflect.deleteProperty(this.settings, 'publish')
      return
    }

    if (!this.settings.publish) {
      this.settings.publish = {}
    }

    this.updateNestedField(
      this.settings.publish,
      'netlifyAccessToken',
      update.publish.netlifyAccessToken,
    )

    this.removeIfEmpty('publish')
  }

  private async handleDataDirChange(update: UpdateSettingsRequest): Promise<void> {
    if (update.dataDir === undefined) {
      return
    }

    const newDataDir = this.getDataDir()
    const newSettingsPath = path.join(newDataDir, 'settings.json')

    if (this.settingsPath && this.settingsPath !== newSettingsPath) {
      await this.moveSettingsFile(this.settingsPath, newSettingsPath, newDataDir)
    }

    this.settingsPath = newSettingsPath
  }

  private async moveSettingsFile(
    oldPath: string,
    newPath: string,
    newDataDir: string,
  ): Promise<void> {
    try {
      await fs.mkdir(newDataDir, { recursive: true })
      await fs.copyFile(oldPath, newPath)
      await fs.unlink(oldPath)

      logger.info('Moved settings file to new location', {
        oldPath,
        newPath,
      })
    } catch (error) {
      logger.error('Failed to move settings file', error as Error, {
        oldPath,
        newPath,
      })
    }
  }

  private updateField<K extends keyof ApplicationSettings>(
    key: K,
    value: ApplicationSettings[K] | null | undefined,
  ): void {
    if (value === undefined) {
      return
    }

    if (!this.settings) {
      return
    }

    if (value === null) {
      Reflect.deleteProperty(this.settings, key)
    } else {
      this.settings[key] = value
    }
  }

  private updateNestedField<T extends object, K extends keyof T>(
    obj: T,
    key: K,
    value: T[K] | null | undefined,
  ): void {
    if (value === undefined) {
      return
    }

    if (value === null) {
      Reflect.deleteProperty(obj, key)
    } else {
      obj[key] = value
    }
  }

  private removeIfEmpty(key: 'claude' | 'mcp' | 'publish'): void {
    if (this.settings?.[key] && Object.keys(this.settings[key]).length === 0) {
      Reflect.deleteProperty(this.settings, key)
    }
  }

  async resetSettings(): Promise<SettingsResponse> {
    this.settings = {}
    await this.saveSettings()
    return this.getSettings()
  }
}
