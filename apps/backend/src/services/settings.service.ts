import { promises as fs } from 'node:fs'
import * as os from 'node:os'
import * as path from 'node:path'
import type {
  ApplicationSettings,
  ClaudeSettings,
  ClaudeSettingsResponse,
  GeneralSettingsResponse,
  KubernetesSettings,
  PublishSettings,
  PublishSettingsResponse,
  SettingsResponse,
  StorageSettings,
  SystemStatusCheck,
  UpdateClaudeSettingsRequest,
  UpdateGeneralSettingsRequest,
  UpdatePublishSettingsRequest,
  UpdateSettingsRequest,
  WorkspaceExecutionMode,
} from '@moatless/types'
import { injectable } from 'inversify'
import { checkSystemStatus } from '../features/claude/agent/claude-utils'
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
      oauthToken: string
    }
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
        oauthToken: process.env['CLAUDE_CODE_OAUTH_TOKEN'] || '',
      },
      publish: {
        netlifyAccessToken: process.env['NETLIFY_ACCESS_TOKEN'] || '',
      },
    }
  }

  async initialize(): Promise<void> {
    try {
      // Load settings from file if it exists
      await this.loadSettings()
    } catch (error) {
      logger.error({ err: error as Error }, 'Failed to initialize settings service')
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
      logger.info(
        {
          settingsPath: this.settingsPath,
        },
        'Loaded settings from file',
      )
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        logger.info(
          {
            settingsPath: this.settingsPath,
          },
          'No settings file found, using defaults',
        )
        this.settings = {}
      } else {
        logger.error(
          { err: error as Error, settingsPath: this.settingsPath },
          'Failed to load settings',
        )
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
    logger.info({ settingsPath: this.settingsPath }, 'Saved settings to file')
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
   * Get default repository visibility from environment variable
   * @returns 'public' if DEFAULT_REPOSITORY_VISIBILITY=public, otherwise 'private'
   */
  getDefaultRepositoryVisibility(): 'private' | 'public' {
    const envValue = process.env['DEFAULT_REPOSITORY_VISIBILITY']
    if (envValue === 'public') {
      return 'public'
    }
    return 'private'
  }

  /**
   * Get default task visibility from environment variable
   * @returns 'public' if DEFAULT_TASK_VISIBILITY=public, otherwise 'private'
   */
  getDefaultTaskVisibility(): 'private' | 'public' {
    const envValue = process.env['DEFAULT_TASK_VISIBILITY']
    if (envValue === 'public') {
      return 'public'
    }
    return 'private'
  }

  /**
   * Get workspace execution mode from environment variable
   * This is a system-wide setting that determines which environment driver to use
   * @returns 'docker' or 'kubernetes' based on WORKSPACE_EXECUTION_MODE env var (defaults to 'kubernetes')
   */
  getWorkspaceExecutionMode(): WorkspaceExecutionMode {
    const mode = process.env['WORKSPACE_EXECUTION_MODE']
    if (mode === 'docker') {
      return 'docker'
    }
    // Default to kubernetes
    return 'kubernetes'
  }

  /**
   * Get Kubernetes configuration settings
   * Settings come from environment variables and can be overridden in settings.json
   */
  getKubernetesSettings(): Required<KubernetesSettings> {
    const settingsK8s = this.settings?.kubernetes
    return {
      enabled: this.getKubeSetting(settingsK8s?.enabled, 'K8S_ENABLED', false),
      namespace: this.getKubeSetting(settingsK8s?.namespace, 'K8S_NAMESPACE', 'moatless'),
      kubeconfig: settingsK8s?.kubeconfig ?? process.env['KUBECONFIG'],
      image: this.getKubeSetting(
        settingsK8s?.image,
        'K8S_SANDBOX_IMAGE',
        'ghcr.io/aorwall/moatless-vibe-sandbox',
      ),
      imagePullSecrets: settingsK8s?.imagePullSecrets,
      serviceAccount: settingsK8s?.serviceAccount ?? process.env['K8S_SERVICE_ACCOUNT'],
      resourceLimits: settingsK8s?.resourceLimits ?? this.getDefaultResourceLimits(),
      resourceRequests: settingsK8s?.resourceRequests ?? this.getDefaultResourceRequests(),
      nodeSelector: settingsK8s?.nodeSelector,
      tolerations: settingsK8s?.tolerations,
      podStartupTimeoutSeconds: this.getKubeIntSetting(
        settingsK8s?.podStartupTimeoutSeconds,
        'K8S_POD_STARTUP_TIMEOUT',
        120,
      ),
      healthCheckIntervalSeconds: this.getKubeIntSetting(
        settingsK8s?.healthCheckIntervalSeconds,
        'K8S_HEALTH_CHECK_INTERVAL',
        30,
      ),
      workerApiUrl: settingsK8s?.workerApiUrl ?? process.env['WORKER_API_URL'],
      workerApiKey: settingsK8s?.workerApiKey ?? process.env['WORKER_API_KEY'],
      workerPodTemplate: settingsK8s?.workerPodTemplate ?? process.env['K8S_WORKER_POD_TEMPLATE'],
      workerStorageEnabled:
        settingsK8s?.workerStorageEnabled ?? process.env['K8S_WORKER_STORAGE_ENABLED'] !== 'false',
      workerStorageClass:
        settingsK8s?.workerStorageClass ?? process.env['K8S_WORKER_STORAGE_CLASS'],
      workerStorageSize:
        settingsK8s?.workerStorageSize ?? process.env['K8S_WORKER_STORAGE_SIZE'] ?? '10Gi',
    }
  }

  private getKubeSetting<T>(settingsValue: T | undefined, envKey: string, defaultValue: T): T {
    return settingsValue ?? (process.env[envKey] as T | undefined) ?? defaultValue
  }

  private getKubeIntSetting(
    settingsValue: number | undefined,
    envKey: string,
    defaultValue: number,
  ): number {
    return settingsValue ?? Number.parseInt(process.env[envKey] ?? defaultValue.toString(), 10)
  }

  private getDefaultResourceLimits() {
    return {
      cpu: process.env['K8S_RESOURCE_LIMIT_CPU'] ?? '2',
      memory: process.env['K8S_RESOURCE_LIMIT_MEMORY'] ?? '4Gi',
    }
  }

  private getDefaultResourceRequests() {
    return {
      cpu: process.env['K8S_RESOURCE_REQUEST_CPU'] ?? '500m',
      memory: process.env['K8S_RESOURCE_REQUEST_MEMORY'] ?? '1Gi',
    }
  }

  getClaudeSettings(): Required<ClaudeSettings> {
    return {
      oauthToken:
        this.settings?.claude?.oauthToken ||
        this.settings?.oauthToken ||
        this.defaults.claude.oauthToken,
    }
  }

  /**
   * Get file storage settings for uploads
   *
   * In Docker mode: Files stored locally, mounted to sandbox via volume
   * In K8s mode: Files stored in S3, mounted to sandbox via CSI driver
   *
   * Environment variables:
   * - FILE_STORAGE_BACKEND: 'local' or 's3' (default: 'local')
   * - S3_BUCKET_NAME: S3 bucket for uploads
   * - S3_REGION: AWS region (default: 'us-east-1')
   * - S3_KEY_PREFIX: Key prefix in bucket (default: 'uploads')
   * - SANDBOX_UPLOAD_MOUNT_PATH: Path where uploads are mounted in sandbox (default: '/mnt/uploads')
   */
  getStorageSettings(): Required<StorageSettings> {
    const backendEnv = process.env['FILE_STORAGE_BACKEND']
    const backend = backendEnv === 's3' ? 's3' : 'local'

    return {
      backend,
      s3BucketName: process.env['S3_BUCKET_NAME'] ?? '',
      s3Region: process.env['S3_REGION'] ?? 'us-east-1',
      s3KeyPrefix: process.env['S3_KEY_PREFIX'] ?? 'uploads',
      sandboxMountPath: process.env['SANDBOX_UPLOAD_MOUNT_PATH'] ?? '/mnt/uploads',
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

  getClaudeSettingsResponse(): ClaudeSettingsResponse {
    const oauthToken = this.settings?.oauthToken || this.defaults.claude.oauthToken

    return {
      oauthToken,
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

        logger.info(
          {
            oldPath: this.settingsPath,
            newPath: newSettingsPath,
          },
          'Moved settings file to new location',
        )
      } catch (error) {
        logger.error(
          { err: error as Error, oldPath: this.settingsPath, newPath: newSettingsPath },
          'Failed to move settings file',
        )
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
      oauthToken: this.settings?.oauthToken,
      claude: this.settings?.claude
        ? {
            oauthToken: this.settings.claude.oauthToken,
          }
        : undefined,
      userGroups: this.settings?.userGroups,
    }

    const fullDefaults: ApplicationSettings = {
      dataDir: this.defaults.dataDir,
      repositoriesDir: this.defaults.repositoriesDir,
      defaultWorkspaceType: this.defaults.defaultWorkspaceType,
      oauthToken: this.defaults.claude.oauthToken,
      claude: {
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

      logger.info(
        {
          oldPath,
          newPath,
        },
        'Moved settings file to new location',
      )
    } catch (error) {
      logger.error({ err: error as Error, oldPath, newPath }, 'Failed to move settings file')
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
