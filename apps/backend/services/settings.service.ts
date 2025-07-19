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
  SystemStatusCheck,
  UpdateClaudeSettingsRequest,
  UpdateGeneralSettingsRequest,
  UpdateGitHubSettingsRequest,
} from '@moatless/types'
import { checkSystemStatus } from '../agents/claude-code/claude-utils.js'
import { getGitUserEmail, getGitUserName } from '../utils/git-utils.js'
import { logger } from '../utils/logger.js'
import type { GitHubService } from './github.service.js'

export class SettingsService {
  private settings: ApplicationSettings | null = null
  private defaults: {
    dataDir: string
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
  }

  constructor(
    private githubService: GitHubService,
    private settingsPath: string | null = null,
  ) {
    // Initialize defaults with placeholder values - will be updated in initialize()
    this.defaults = {
      dataDir: process.env.DATA_DIR || path.join(os.homedir(), '.moatless'),
      defaultWorkspaceType: (process.env.DEFAULT_WORKSPACE_TYPE === 'docker'
        ? 'docker'
        : 'local') as 'docker' | 'local',
      claude: {
        binDir: process.env.CLAUDE_BIN_DIR || '',
        configDir: process.env.CLAUDE_CONFIG_DIR || '',
        oauthToken: process.env.CLAUDE_CODE_OAUTH_TOKEN || '',
      },
      github: {
        personalAccessToken: process.env.GITHUB_TOKEN || '',
        userName: '',
        userEmail: '',
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
      binDir: this.settings?.binDir || this.defaults.claude.binDir,
      configDir: this.settings?.configDir || this.defaults.claude.configDir,
      oauthToken: this.settings?.oauthToken || this.defaults.claude.oauthToken,
    }
  }

  getGitHubSettings(): Required<GitHubSettings> {
    return {
      personalAccessToken:
        this.settings?.github?.personalAccessToken || this.defaults.github.personalAccessToken,
      userName: this.settings?.github?.userName || this.defaults.github.userName,
      userEmail: this.settings?.github?.userEmail || this.defaults.github.userEmail,
    }
  }

  async getGeneralSettings(): Promise<GeneralSettingsResponse> {
    return {
      settings: {
        dataDir: this.settings?.dataDir,
        defaultWorkspaceType: this.settings?.defaultWorkspaceType,
      },
      defaults: {
        dataDir: this.defaults.dataDir,
        defaultWorkspaceType: this.defaults.defaultWorkspaceType,
      },
    }
  }

  async getClaudeSettingsResponse(): Promise<ClaudeSettingsResponse> {
    return {
      binDir: this.settings?.binDir || this.defaults.claude.binDir,
      configDir: this.settings?.configDir || this.defaults.claude.configDir,
      oauthToken: this.settings?.oauthToken || this.defaults.claude.oauthToken,
    }
  }

  async getGitHubSettingsResponse(): Promise<GitHubSettingsResponse> {
    return {
      settings: {
        personalAccessToken: this.settings?.github?.personalAccessToken,
        userName: this.settings?.github?.userName,
        userEmail: this.settings?.github?.userEmail,
      },
      defaults: this.defaults.github,
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
        delete this.settings.binDir
      } else {
        this.settings.binDir = update.binDir
      }
    }

    if (update.configDir !== undefined) {
      if (update.configDir === null) {
        delete this.settings.configDir
      } else {
        this.settings.configDir = update.configDir
      }
    }

    if (update.oauthToken !== undefined) {
      if (update.oauthToken === null) {
        delete this.settings.oauthToken
      } else {
        this.settings.oauthToken = update.oauthToken
      }
    }

    // Save updated settings
    await this.saveSettings()

    return this.getClaudeSettingsResponse()
  }

  async updateGitHubSettingsSection(
    update: UpdateGitHubSettingsRequest,
  ): Promise<GitHubSettingsResponse> {
    if (!this.settings) {
      this.settings = {}
    }

    // Update GitHub settings
    if (!this.settings.github) {
      this.settings.github = {}
    }

    let shouldFetchUserInfo = false

    if (update.personalAccessToken !== undefined) {
      if (update.personalAccessToken === null) {
        delete this.settings.github.personalAccessToken
        // Clear user info when token is removed
        delete this.settings.github.userName
        delete this.settings.github.userEmail
      } else {
        this.settings.github.personalAccessToken = update.personalAccessToken
        shouldFetchUserInfo = true
      }
    }

    // If a new PAT was provided, fetch user info from GitHub
    if (shouldFetchUserInfo && this.settings.github.personalAccessToken) {
      const githubUser = await this.githubService.getAuthenticatedUser(
        this.settings.github.personalAccessToken,
      )

      if (githubUser) {
        // Use the GitHub username as the git user name
        this.settings.github.userName = githubUser.name || githubUser.login

        // Use the GitHub email if available
        if (githubUser.email) {
          this.settings.github.userEmail = githubUser.email
        }

        logger.info('Fetched GitHub user info', {
          userName: this.settings.github.userName,
          userEmail: this.settings.github.userEmail,
        })
      } else {
        logger.warn('Failed to fetch GitHub user info, using git config defaults')
      }
    } else {
      // Allow manual override of userName and userEmail
      if (update.userName !== undefined) {
        if (update.userName === null) {
          delete this.settings.github.userName
        } else {
          this.settings.github.userName = update.userName
        }
      }

      if (update.userEmail !== undefined) {
        if (update.userEmail === null) {
          delete this.settings.github.userEmail
        } else {
          this.settings.github.userEmail = update.userEmail
        }
      }
    }

    // Remove github object if empty
    if (Object.keys(this.settings.github).length === 0) {
      delete this.settings.github
    }

    // Save updated settings
    await this.saveSettings()

    return this.getGitHubSettingsResponse()
  }

  async getSystemStatus(): Promise<SystemStatusCheck> {
    return checkSystemStatus()
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
