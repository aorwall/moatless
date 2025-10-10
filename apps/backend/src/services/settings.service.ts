import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type {
  ApplicationSettings,
  ClaudeSettings,
  ClaudeSettingsResponse,
  GeneralSettingsResponse,
  GitHubSettings,
  GitHubSettingsResponse,
  PublishSettings,
  PublishSettingsResponse,
  SettingsResponse,
  SystemStatusCheck,
  UpdateClaudeSettingsRequest,
  UpdateGeneralSettingsRequest,
  UpdateGitHubSettingsRequest,
  UpdatePublishSettingsRequest,
  UpdateSettingsRequest,
} from '@moatless/types'
import { checkSystemStatus } from '../agents/claude-code/claude-utils.js'
import { getGitUserEmail, getGitUserName } from '../utils/git-utils.js'
import { logger } from '../utils/logger.js'
import type { GitHubService } from './github.service.js'

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
    github: {
      personalAccessToken: string
      userName: string
      userEmail: string
    }
    publish: {
      netlifyAccessToken: string
    }
  }

  constructor(
    private githubService: GitHubService,
    private settingsPath: string | null = null,
  ) {
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
      github: {
        personalAccessToken: process.env['GITHUB_TOKEN'] || '',
        userName: '',
        userEmail: '',
      },
      publish: {
        netlifyAccessToken: process.env['NETLIFY_ACCESS_TOKEN'] || '',
      },
    }
  }

  async initialize(): Promise<void> {
    try {
      // Read git config values
      const [gitUserName, gitUserEmail] = await Promise.all([getGitUserName(), getGitUserEmail()])

      // Update defaults with git config values
      this.defaults.github.userName = gitUserName || ''
      this.defaults.github.userEmail = gitUserEmail || ''

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
      const { ClaudeService } = await import('../agents/claude-code/claude.service.js')

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

  getGitHubSettings(sessionToken?: string): Required<GitHubSettings> {
    // Prefer OAuth session token over configured PAT
    const token =
      sessionToken ||
      this.settings?.github?.personalAccessToken ||
      this.defaults.github.personalAccessToken

    return {
      personalAccessToken: token,
      userName: this.settings?.github?.userName || this.defaults.github.userName,
      userEmail: this.settings?.github?.userEmail || this.defaults.github.userEmail,
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
        const { ClaudeService } = await import('../agents/claude-code/claude.service.js')
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

  getGitHubSettingsResponse(): GitHubSettingsResponse {
    return {
      settings: {
        personalAccessToken: this.settings?.github?.personalAccessToken,
        userName: this.settings?.github?.userName,
        userEmail: this.settings?.github?.userEmail,
      },
      defaults: this.defaults.github,
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

  private updatePersonalAccessToken(
    update: UpdateGitHubSettingsRequest,
    githubSettings: GitHubSettings,
  ): boolean {
    if (update.personalAccessToken === undefined) {
      return false
    }

    if (update.personalAccessToken === null) {
      Reflect.deleteProperty(githubSettings, 'personalAccessToken')
      Reflect.deleteProperty(githubSettings, 'userName')
      Reflect.deleteProperty(githubSettings, 'userEmail')
      return false
    }

    githubSettings.personalAccessToken = update.personalAccessToken
    return update.userName === undefined && update.userEmail === undefined
  }

  private async fetchGitHubUserAndUpdate(
    personalAccessToken: string,
    githubSettings: GitHubSettings,
  ): Promise<void> {
    const githubUser = await this.githubService.getAuthenticatedUser(personalAccessToken)

    if (!githubUser) {
      logger.warn('Failed to fetch GitHub user info, using git config defaults')
      return
    }

    githubSettings.userName = githubUser.name || githubUser.login

    if (githubUser.email) {
      githubSettings.userEmail = githubUser.email
    }

    logger.info('Fetched GitHub user info', {
      userName: githubSettings.userName,
      userEmail: githubSettings.userEmail,
    })
  }

  private updateGitHubUserField(
    fieldName: 'userName' | 'userEmail',
    value: string | null | undefined,
    githubSettings: GitHubSettings,
  ): void {
    if (value === undefined) {
      return
    }

    if (value === null) {
      Reflect.deleteProperty(githubSettings, fieldName)
    } else {
      githubSettings[fieldName] = value
    }
  }

  async updateGitHubSettingsSection(
    update: UpdateGitHubSettingsRequest,
  ): Promise<GitHubSettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    if (!this.settings.github) {
      this.settings.github = {}
    }

    const shouldFetchUserInfo = this.updatePersonalAccessToken(update, this.settings.github)

    if (shouldFetchUserInfo && this.settings.github.personalAccessToken) {
      await this.fetchGitHubUserAndUpdate(
        this.settings.github.personalAccessToken,
        this.settings.github,
      )
    }

    this.updateGitHubUserField('userName', update.userName, this.settings.github)
    this.updateGitHubUserField('userEmail', update.userEmail, this.settings.github)

    if (Object.keys(this.settings.github).length === 0) {
      Reflect.deleteProperty(this.settings, 'github')
    }

    await this.saveSettings()

    return this.getGitHubSettingsResponse()
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
      github: this.settings?.github
        ? {
            personalAccessToken: this.settings.github.personalAccessToken,
            userName: this.settings.github.userName,
            userEmail: this.settings.github.userEmail,
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
      github: {
        personalAccessToken: this.defaults.github.personalAccessToken,
        userName: this.defaults.github.userName,
        userEmail: this.defaults.github.userEmail,
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
    await this.updateGitHubSettings(update)
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

  private async updateGitHubSettings(update: UpdateSettingsRequest): Promise<void> {
    if (update.github === undefined || !this.settings) {
      return
    }

    if (update.github === null) {
      Reflect.deleteProperty(this.settings, 'github')
      return
    }

    if (!this.settings.github) {
      this.settings.github = {}
    }

    const shouldFetchUserInfo = await this.updateGitHubToken(update.github.token)

    if (shouldFetchUserInfo) {
      await this.fetchAndStoreGitHubUserInfo()
    }

    this.removeIfEmpty('github')
  }

  private updateGitHubToken(token: string | null | undefined): boolean {
    if (token === undefined || !this.settings) {
      return false
    }

    if (token === null) {
      if (this.settings.github) {
        Reflect.deleteProperty(this.settings.github, 'personalAccessToken')
        Reflect.deleteProperty(this.settings.github, 'userName')
        Reflect.deleteProperty(this.settings.github, 'userEmail')
      }
      return false
    }

    if (this.settings.github) {
      this.settings.github.personalAccessToken = token
    }
    return true
  }

  private async fetchAndStoreGitHubUserInfo(): Promise<void> {
    if (!this.settings?.github?.personalAccessToken) {
      return
    }

    const githubUser = await this.githubService.getAuthenticatedUser(
      this.settings.github.personalAccessToken,
    )

    if (githubUser) {
      this.storeGitHubUserInfo(githubUser)
    } else {
      logger.warn('Failed to fetch GitHub user info, using git config defaults')
    }
  }

  private storeGitHubUserInfo(githubUser: {
    name?: string | null
    login: string
    email?: string | null
  }): void {
    if (!this.settings?.github) {
      return
    }

    this.settings.github.userName = githubUser.name || githubUser.login

    if (githubUser.email) {
      this.settings.github.userEmail = githubUser.email
    }

    logger.info('Fetched GitHub user info', {
      userName: this.settings.github.userName,
      userEmail: this.settings.github.userEmail,
    })
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

  private removeIfEmpty(key: 'claude' | 'github' | 'publish'): void {
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

// Factory function for dependency injection
export async function createSettingsService(
  githubService: GitHubService,
  settingsPath?: string,
): Promise<SettingsService> {
  const service = new SettingsService(githubService, settingsPath)
  await service.initialize()
  return service
}

// Singleton instance
let settingsServiceInstance: SettingsService | null = null

export function getSettingsService(githubService: GitHubService): SettingsService {
  if (!settingsServiceInstance) {
    settingsServiceInstance = new SettingsService(githubService)
    // Note: We can't call initialize() here as it's async
    // This is for compatibility with sync usage - caller needs to handle initialization
  }
  return settingsServiceInstance
}

// For testing: reset the singleton instance
export function resetSettingsServiceInstance(): void {
  settingsServiceInstance = null
}

// Initialize settings service with GitHub service - for integration tests
export async function initializeSettingsService(): Promise<SettingsService> {
  // Create a mock GitHubService for integration tests
  const githubService = {
    apiBaseUrl: 'https://api.github.com',
    getAuthenticatedUser: async () => null,
    getUserEmails: async () => null,
    getUserRepositories: async () => [],
    validateToken: async () => false,
    getTokenScopes: async () => null,
  } as unknown as GitHubService

  return await createSettingsService(githubService)
}
