import { exec } from 'node:child_process'
import { existsSync, promises as fs } from 'node:fs'
import * as path from 'node:path'
import { promisify } from 'node:util'
import type {
  MarketplaceManifest,
  MarketplacePluginEntry,
  PluginInfo,
  PluginMarketplace,
} from '@moatless/types'
import { MarketplaceManifestSchema } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { v4 as uuidv4 } from 'uuid'
import { SettingsService } from '../../../services/settings.service'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import { S3StorageBackend } from '../../file-storage/backends/s3-storage-backend'
import type { StorageBackend } from '../../file-storage/backends/storage-backend'
import { PluginMarketplaceRepository } from '../db/plugin-marketplace.repository'

const execAsync = promisify(exec)

/**
 * Service for managing plugin marketplaces and resolving plugin paths.
 * Handles cloning, syncing, and path resolution for Claude Code plugins.
 */
@injectable()
export class PluginService {
  private readonly pluginsDir: string
  private readonly marketplacesDir: string
  private readonly sandboxPluginsDir: string
  private readonly pluginLogger = logger.child({ service: 'PluginService' })
  private storageBackend: StorageBackend | null = null

  constructor(
    @inject(PluginMarketplaceRepository) private readonly repository: PluginMarketplaceRepository,
    @inject(SettingsService) private readonly settingsService: SettingsService,
  ) {
    const dataDir = getDataDir()
    this.pluginsDir = process.env['PLUGINS_DIR'] || path.join(dataDir, 'plugins')
    this.marketplacesDir = path.join(this.pluginsDir, 'marketplaces')
    // Mount at CLAUDE_CONFIG_DIR/plugins for auto-discovery by Claude Code
    this.sandboxPluginsDir = process.env['SANDBOX_PLUGINS_DIR'] || '/opt/moatless/.claude/plugins'
  }

  /**
   * Initialize the plugin service.
   * Ensures the S3 plugins prefix exists and re-syncs any marketplaces
   * that are marked as synced but missing from S3.
   */
  async initialize(): Promise<void> {
    await this.ensurePluginsPrefixExists()

    const backend = this.getStorageBackend()
    if (!backend) {
      return
    }

    // Check synced marketplaces and re-sync if missing from S3
    const marketplaces = await this.repository.getAll()
    for (const marketplace of marketplaces) {
      if (marketplace.status !== 'synced') {
        continue
      }
      const repoName = this.extractRepoName(marketplace.repoUrl)
      try {
        const s3Objects = await backend.list(`marketplaces/${repoName}/`)
        if (s3Objects.length > 0) {
          this.pluginLogger.debug(
            { repoName, objectCount: s3Objects.length },
            'Marketplace already in S3',
          )
          continue
        }

        const marketplaceDir = path.join(this.marketplacesDir, repoName)
        if (existsSync(marketplaceDir)) {
          // Local files exist but not in S3 — upload them
          this.pluginLogger.info({ repoName }, 'Marketplace missing from S3, syncing from local')
          await this.syncMarketplaceToS3(marketplaceDir, repoName)
        } else {
          // Neither local nor S3 — re-clone then sync (e.g. pod restart with ephemeral disk)
          this.pluginLogger.info({ repoName }, 'Marketplace missing from S3 and local, re-cloning')
          await this.syncMarketplace(marketplace.id)
        }
      } catch (error) {
        this.pluginLogger.error(
          { err: error, repoName },
          'Failed to re-sync marketplace to S3 during initialization',
        )
      }
    }
  }

  /**
   * Get the plugins directory path.
   */
  getPluginsDir(): string {
    return this.pluginsDir
  }

  /**
   * Add a new marketplace by cloning the repository.
   *
   * @param repoUrl - GitHub repository URL
   * @returns Created plugin marketplace
   * @throws Error if cloning fails or marketplace.json is invalid
   */
  async addMarketplace(repoUrl: string): Promise<PluginMarketplace> {
    // Normalize URL (strip /tree/..., /blob/..., etc.)
    const normalizedUrl = this.normalizeGitHubUrl(repoUrl)
    this.pluginLogger.info({ repoUrl, normalizedUrl }, 'Adding plugin marketplace')

    // Ensure marketplaces directory exists
    await fs.mkdir(this.marketplacesDir, { recursive: true })

    // Extract repo name from URL for directory name
    const repoName = this.extractRepoName(normalizedUrl)
    const marketplaceDir = path.join(this.marketplacesDir, repoName)

    // Check if directory already exists
    if (existsSync(marketplaceDir)) {
      // Check if it's already in the database
      const existing = await this.repository.getByName(repoName)
      if (existing) {
        this.pluginLogger.info({ repoName }, 'Marketplace already exists, syncing instead')
        return this.syncMarketplace(existing.id)
      }
      // Directory exists but not in DB - remove and re-clone
      await fs.rm(marketplaceDir, { recursive: true, force: true })
    }

    // Clone the repository
    try {
      await this.cloneMarketplace(normalizedUrl, marketplaceDir)
    } catch (error) {
      // Clone failed - save marketplace with failed status
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.pluginLogger.error({ err: error, repoUrl: normalizedUrl }, 'Failed to clone marketplace')

      const failedMarketplace: PluginMarketplace = {
        id: uuidv4(),
        name: repoName,
        repoUrl: normalizedUrl,
        status: 'failed',
        lastError: errorMessage,
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      await this.repository.save(failedMarketplace)
      return failedMarketplace
    }

    // Discover plugins using multiple strategies
    let plugins: MarketplacePluginEntry[] = []
    try {
      plugins = await this.discoverPlugins(marketplaceDir)
    } catch (error) {
      // Plugin discovery failed - save marketplace with failed status
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.pluginLogger.error({ err: error, marketplaceDir }, 'Failed to discover plugins')

      const failedMarketplace: PluginMarketplace = {
        id: uuidv4(),
        name: repoName,
        repoUrl: normalizedUrl,
        status: 'failed',
        lastError: errorMessage,
        createdAt: new Date(),
        updatedAt: new Date(),
      }

      await this.repository.save(failedMarketplace)
      return failedMarketplace
    }

    // Try to read marketplace.json for additional metadata, but don't fail if not present
    let manifestName: string | undefined
    let ownerName: string | undefined
    let description: string | undefined

    const manifestPath = path.join(marketplaceDir, '.claude-plugin', 'marketplace.json')
    if (existsSync(manifestPath)) {
      try {
        const manifest = await this.readMarketplaceManifest(marketplaceDir)
        manifestName = manifest.name
        ownerName = manifest.owner?.name
        description = manifest.metadata?.description
      } catch {
        // Ignore - we can proceed without full manifest metadata
      }
    }

    // For single-plugin repos, use the plugin name as marketplace name
    const marketplaceName =
      manifestName || (plugins.length === 1 ? plugins[0]?.name : undefined) || repoName

    // Create marketplace record with synced status
    const marketplace: PluginMarketplace = {
      id: uuidv4(),
      name: marketplaceName,
      repoUrl: normalizedUrl,
      ownerName,
      description: description || plugins[0]?.description,
      status: 'synced',
      lastSyncedAt: new Date(),
      createdAt: new Date(),
      updatedAt: new Date(),
    }

    // Sync to S3 if configured
    try {
      await this.syncMarketplaceToS3(marketplaceDir, repoName)
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.pluginLogger.error({ err: error, repoName }, 'Failed to sync marketplace to S3')
      marketplace.status = 'failed'
      marketplace.lastError = `S3 sync failed: ${errorMessage}`
    }

    await this.repository.save(marketplace)

    this.pluginLogger.info(
      { marketplaceId: marketplace.id, name: marketplace.name },
      'Plugin marketplace added successfully',
    )

    return marketplace
  }

  /**
   * Sync a marketplace by pulling latest changes from remote.
   *
   * @param id - Marketplace identifier
   * @returns Updated plugin marketplace
   * @throws Error if marketplace not found or sync fails
   */
  async syncMarketplace(id: string): Promise<PluginMarketplace> {
    const marketplace = await this.repository.get(id)
    if (!marketplace) {
      throw new Error(`Marketplace not found: ${id}`)
    }

    // Set status to syncing
    const syncingMarketplace: PluginMarketplace = {
      ...marketplace,
      status: 'syncing',
      updatedAt: new Date(),
    }
    await this.repository.save(syncingMarketplace)

    const marketplaceDir = path.join(
      this.marketplacesDir,
      this.extractRepoName(marketplace.repoUrl),
    )

    try {
      if (!existsSync(marketplaceDir)) {
        // Directory was deleted - re-clone
        this.pluginLogger.info({ id }, 'Marketplace directory missing, re-cloning')
        await this.cloneMarketplace(marketplace.repoUrl, marketplaceDir)
      } else {
        // Pull latest changes
        this.pluginLogger.info({ id }, 'Pulling latest changes for marketplace')
        await execAsync('git fetch --depth=1 origin && git reset --hard origin/HEAD', {
          cwd: marketplaceDir,
          timeout: 60000, // 60 seconds timeout
        })
      }

      // Verify plugins can be discovered
      await this.discoverPlugins(marketplaceDir)
    } catch (error) {
      // Sync failed - update status to failed
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.pluginLogger.error({ err: error, id }, 'Failed to sync marketplace')

      const failedMarketplace: PluginMarketplace = {
        ...marketplace,
        status: 'failed',
        lastError: errorMessage,
        updatedAt: new Date(),
      }
      await this.repository.save(failedMarketplace)

      throw new Error(`Failed to sync marketplace: ${errorMessage}`)
    }

    // Try to read manifest for metadata (optional)
    let ownerName: string | undefined
    let description: string | undefined

    const manifestPath = path.join(marketplaceDir, '.claude-plugin', 'marketplace.json')
    if (existsSync(manifestPath)) {
      try {
        const manifest = await this.readMarketplaceManifest(marketplaceDir)
        ownerName = manifest.owner?.name
        description = manifest.metadata?.description
      } catch {
        // Ignore - use existing metadata
        ownerName = marketplace.ownerName
        description = marketplace.description
      }
    } else {
      // Keep existing metadata for non-manifest repos
      ownerName = marketplace.ownerName
      description = marketplace.description
    }

    // Update marketplace record with synced status
    const updatedMarketplace: PluginMarketplace = {
      ...marketplace,
      ownerName,
      description,
      status: 'synced',
      lastError: undefined,
      lastSyncedAt: new Date(),
      updatedAt: new Date(),
    }

    // Sync to S3 if configured
    const repoName = this.extractRepoName(marketplace.repoUrl)
    try {
      await this.syncMarketplaceToS3(marketplaceDir, repoName)
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      this.pluginLogger.error({ err: error, repoName }, 'Failed to sync marketplace to S3')
      updatedMarketplace.status = 'failed'
      updatedMarketplace.lastError = `S3 sync failed: ${errorMessage}`
    }

    await this.repository.save(updatedMarketplace)

    this.pluginLogger.info({ id }, 'Marketplace synced successfully')

    return updatedMarketplace
  }

  /**
   * List all plugin marketplaces.
   *
   * @returns Array of plugin marketplaces
   */
  listMarketplaces(): Promise<PluginMarketplace[]> {
    return this.repository.getAll()
  }

  /**
   * List all plugins from all marketplaces.
   *
   * @returns Array of plugin info objects
   */
  async listPlugins(): Promise<PluginInfo[]> {
    const marketplaces = await this.repository.getAll()
    const plugins: PluginInfo[] = []

    for (const marketplace of marketplaces) {
      // Skip failed marketplaces - they have no valid plugins
      if (marketplace.status === 'failed') {
        this.pluginLogger.debug({ marketplaceId: marketplace.id }, 'Skipping failed marketplace')
        continue
      }

      const marketplaceDir = path.join(
        this.marketplacesDir,
        this.extractRepoName(marketplace.repoUrl),
      )

      if (!existsSync(marketplaceDir)) {
        this.pluginLogger.warn(
          { marketplaceId: marketplace.id },
          'Marketplace directory missing, attempting re-clone',
        )
        this.syncMarketplace(marketplace.id).catch((err) => {
          this.pluginLogger.error(
            { err, marketplaceId: marketplace.id },
            'Failed to re-clone marketplace',
          )
        })
        continue
      }

      try {
        const discoveredPlugins = await this.discoverPlugins(marketplaceDir)

        for (const pluginEntry of discoveredPlugins) {
          plugins.push({
            id: `${pluginEntry.name}@${marketplace.name}`,
            name: pluginEntry.name,
            marketplaceName: marketplace.name,
            marketplaceId: marketplace.id,
            description: pluginEntry.description,
            version: pluginEntry.version,
            source: pluginEntry.source,
          })
        }
      } catch (error) {
        this.pluginLogger.error(
          { err: error, marketplaceId: marketplace.id },
          'Failed to read plugins from marketplace',
        )
      }
    }

    return plugins
  }

  /**
   * Resolve plugin identifier to absolute path (for backend use).
   *
   * @param pluginId - Plugin identifier in format: {plugin-name}@{marketplace-name}
   * @returns Absolute path to plugin or null if not found
   */
  async getPluginPath(pluginId: string): Promise<string | null> {
    const [pluginName, marketplaceName] = pluginId.split('@')
    if (!(pluginName && marketplaceName)) {
      this.pluginLogger.warn({ pluginId }, 'Invalid plugin identifier format')
      return null
    }

    const marketplace = await this.repository.getByName(marketplaceName)
    if (!marketplace) {
      this.pluginLogger.warn({ marketplaceName }, 'Marketplace not found')
      return null
    }

    const marketplaceDir = path.join(
      this.marketplacesDir,
      this.extractRepoName(marketplace.repoUrl),
    )
    if (!existsSync(marketplaceDir)) {
      this.pluginLogger.warn({ marketplaceDir }, 'Marketplace directory not found')
      return null
    }

    try {
      const discoveredPlugins = await this.discoverPlugins(marketplaceDir)
      const pluginEntry = discoveredPlugins.find((p) => p.name === pluginName)

      if (pluginEntry?.source) {
        // Use source field from discovery
        const pluginPath = path.resolve(marketplaceDir, pluginEntry.source)
        if (existsSync(pluginPath)) {
          return pluginPath
        }
      }

      // Fallback: check plugins/{name}
      const fallbackPath = path.join(marketplaceDir, 'plugins', pluginName)
      if (existsSync(fallbackPath)) {
        return fallbackPath
      }

      // Fallback: check root (for single-plugin repos)
      if (existsSync(path.join(marketplaceDir, 'plugin.json'))) {
        return marketplaceDir
      }
    } catch (error) {
      this.pluginLogger.error({ err: error, pluginId }, 'Failed to resolve plugin path')
    }

    return null
  }

  /**
   * Resolve plugin identifier to relative path (for sandbox mount).
   *
   * @param pluginId - Plugin identifier in format: {plugin-name}@{marketplace-name}
   * @returns Relative path from plugins dir or null if not found
   */
  async getPluginRelativePath(pluginId: string): Promise<string | null> {
    const absolutePath = await this.getPluginPath(pluginId)
    if (!absolutePath) {
      return null
    }

    // Return path relative to plugins directory
    return path.relative(this.pluginsDir, absolutePath)
  }

  /**
   * Resolve plugin identifier to sandbox-absolute path.
   *
   * @param pluginId - Plugin identifier in format: {plugin-name}@{marketplace-name}
   * @returns Absolute path within sandbox environment or null if not found
   */
  async getPluginSandboxPath(pluginId: string): Promise<string | null> {
    const relativePath = await this.getPluginRelativePath(pluginId)
    if (!relativePath) {
      return null
    }

    return path.join(this.sandboxPluginsDir, relativePath)
  }

  /**
   * Remove a marketplace and delete its files.
   *
   * @param id - Marketplace identifier
   */
  async removeMarketplace(id: string): Promise<void> {
    const marketplace = await this.repository.get(id)
    if (!marketplace) {
      throw new Error(`Marketplace not found: ${id}`)
    }

    // Delete from filesystem
    const repoName = this.extractRepoName(marketplace.repoUrl)
    const marketplaceDir = path.join(this.marketplacesDir, repoName)
    if (existsSync(marketplaceDir)) {
      await fs.rm(marketplaceDir, { recursive: true, force: true })
    }

    // Delete from S3 if configured
    await this.deleteMarketplaceFromS3(repoName)

    // Delete from database
    await this.repository.delete(id)

    this.pluginLogger.info({ id }, 'Marketplace removed successfully')
  }

  /**
   * Clone a marketplace repository with sparse checkout.
   */
  private async cloneMarketplace(repoUrl: string, targetDir: string): Promise<void> {
    this.pluginLogger.info({ repoUrl, targetDir }, 'Cloning marketplace repository')

    try {
      // Shallow clone: downloads only the latest commit with full working tree.
      // We need all files checked out because root-based plugins (source: ".")
      // have skills/, commands/, agents/ etc. at the repo root.
      await execAsync(`git clone --depth=1 "${repoUrl}" "${targetDir}"`, {
        timeout: 120000, // 2 minutes timeout
      })
    } catch (error) {
      // Clean up on failure
      if (existsSync(targetDir)) {
        await fs.rm(targetDir, { recursive: true, force: true })
      }
      throw new Error(
        `Failed to clone marketplace: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Discover plugins in a marketplace directory using multiple strategies.
   *
   * Strategy 1: Explicit marketplace.json in .claude-plugin/
   * Strategy 2: Single plugin (plugin.json in root)
   * Strategy 3: Multi-plugin directory (plugins/ with subdirectories)
   *
   * @param marketplaceDir - Path to the marketplace directory
   * @returns Array of discovered plugin entries
   */
  private async discoverPlugins(marketplaceDir: string): Promise<MarketplacePluginEntry[]> {
    // Strategy 1: Explicit marketplace.json
    const manifestPath = path.join(marketplaceDir, '.claude-plugin', 'marketplace.json')
    if (existsSync(manifestPath)) {
      this.pluginLogger.debug({ marketplaceDir }, 'Using Strategy 1: marketplace.json')
      const manifest = await this.readMarketplaceManifest(marketplaceDir)
      return manifest.plugins
    }

    // Strategy 2: Single plugin (plugin.json in root)
    const rootPluginJson = path.join(marketplaceDir, 'plugin.json')
    if (existsSync(rootPluginJson)) {
      this.pluginLogger.debug({ marketplaceDir }, 'Using Strategy 2: root plugin.json')
      return this.discoverSinglePlugin(marketplaceDir, rootPluginJson)
    }

    // Strategy 3: plugins/ directory with subdirectories
    const pluginsDir = path.join(marketplaceDir, 'plugins')
    if (existsSync(pluginsDir)) {
      this.pluginLogger.debug({ marketplaceDir }, 'Using Strategy 3: plugins/ directory')
      const plugins = await this.discoverPluginsInDirectory(pluginsDir)
      if (plugins.length > 0) {
        return plugins
      }
    }

    throw new Error(`No plugins found in marketplace: ${marketplaceDir}`)
  }

  /**
   * Discover a single plugin from a root plugin.json file.
   */
  private async discoverSinglePlugin(
    marketplaceDir: string,
    pluginJsonPath: string,
  ): Promise<MarketplacePluginEntry[]> {
    const content = await fs.readFile(pluginJsonPath, 'utf-8')
    let pluginManifest: { name?: string; description?: string; version?: string }
    try {
      pluginManifest = JSON.parse(content)
    } catch {
      throw new Error(`Invalid JSON in plugin.json: ${pluginJsonPath}`)
    }
    return [
      {
        name: pluginManifest.name || path.basename(marketplaceDir),
        description: pluginManifest.description,
        version: pluginManifest.version,
        source: '.', // root directory
      },
    ]
  }

  /**
   * Discover plugins from subdirectories in a plugins/ directory.
   */
  private async discoverPluginsInDirectory(pluginsDir: string): Promise<MarketplacePluginEntry[]> {
    const entries = await fs.readdir(pluginsDir, { withFileTypes: true })
    const plugins: MarketplacePluginEntry[] = []

    for (const entry of entries) {
      if (!entry.isDirectory()) {
        continue
      }
      const pluginJsonPath = path.join(pluginsDir, entry.name, 'plugin.json')
      if (!existsSync(pluginJsonPath)) {
        continue
      }
      const pluginEntry = await this.parsePluginJson(pluginJsonPath, entry.name)
      if (pluginEntry) {
        plugins.push(pluginEntry)
      }
    }

    return plugins
  }

  /**
   * Parse a plugin.json file and return a plugin entry.
   */
  private async parsePluginJson(
    pluginJsonPath: string,
    dirName: string,
  ): Promise<MarketplacePluginEntry | null> {
    const content = await fs.readFile(pluginJsonPath, 'utf-8')
    let manifest: { name?: string; description?: string; version?: string }
    try {
      manifest = JSON.parse(content)
    } catch {
      this.pluginLogger.warn({ pluginJsonPath }, 'Invalid JSON in plugin.json, skipping')
      return null
    }
    return {
      name: manifest.name || dirName,
      description: manifest.description,
      version: manifest.version,
      source: `plugins/${dirName}`,
    }
  }

  /**
   * Read and parse marketplace.json from a marketplace directory.
   */
  private async readMarketplaceManifest(marketplaceDir: string): Promise<MarketplaceManifest> {
    const manifestPath = path.join(marketplaceDir, '.claude-plugin', 'marketplace.json')

    if (!existsSync(manifestPath)) {
      throw new Error(`Marketplace manifest not found: ${manifestPath}`)
    }

    const content = await fs.readFile(manifestPath, 'utf-8')
    let parsed: unknown

    try {
      parsed = JSON.parse(content)
    } catch {
      throw new Error(`Invalid JSON in marketplace manifest: ${manifestPath}`)
    }

    const result = MarketplaceManifestSchema.safeParse(parsed)
    if (!result.success) {
      throw new Error(`Invalid marketplace manifest: ${result.error.message}`)
    }

    return result.data
  }

  /**
   * Normalize a GitHub URL to the base repository URL.
   * Strips /tree/..., /blob/..., and other path suffixes.
   *
   * Examples:
   * - https://github.com/owner/repo/tree/main -> https://github.com/owner/repo
   * - https://github.com/owner/repo/blob/main/file.txt -> https://github.com/owner/repo
   * - https://github.com/owner/repo -> https://github.com/owner/repo (unchanged)
   */
  private normalizeGitHubUrl(repoUrl: string): string {
    // Match GitHub URLs and extract owner/repo
    const match = repoUrl.match(
      /^(https?:\/\/github\.com\/[^/]+\/[^/]+)(?:\/(?:tree|blob|commits|pull|issues|actions|releases|tags|branches)\/.*)?$/,
    )
    if (match?.[1]) {
      return match[1]
    }

    // Return as-is if not a recognized GitHub URL pattern
    return repoUrl
  }

  /**
   * Extract repository name from GitHub URL.
   */
  private extractRepoName(repoUrl: string): string {
    // First normalize the URL
    const normalizedUrl = this.normalizeGitHubUrl(repoUrl)

    // Handle various URL formats:
    // https://github.com/owner/repo
    // https://github.com/owner/repo.git
    // git@github.com:owner/repo.git
    const match = normalizedUrl.match(/\/([^/]+?)(?:\.git)?$/)
    if (match?.[1]) {
      return match[1]
    }

    // Fallback: use last segment
    const segments = normalizedUrl.split('/')
    return segments[segments.length - 1]?.replace('.git', '') || 'unknown'
  }

  /**
   * Get the storage backend for plugins (S3 or local), lazily initialized.
   * Returns null if storage backend is local (no sync needed).
   */
  private getStorageBackend(): StorageBackend | null {
    if (this.storageBackend) {
      return this.storageBackend
    }

    const storageSettings = this.settingsService.getStorageSettings()
    if (storageSettings.backend !== 's3') {
      return null
    }

    if (!storageSettings.s3BucketName) {
      this.pluginLogger.warn('S3_BUCKET_NAME not set, skipping S3 plugin sync')
      return null
    }

    this.storageBackend = new S3StorageBackend(
      storageSettings.s3BucketName,
      'plugins',
      storageSettings.s3Region,
    )
    this.pluginLogger.info(
      { bucket: storageSettings.s3BucketName, keyPrefix: 'plugins' },
      'S3 storage backend initialized for plugins',
    )

    return this.storageBackend
  }

  /**
   * Ensure the S3 plugins/ prefix exists by writing a zero-byte .keep file.
   * This prevents PVC subPath mount failures when no marketplaces are configured.
   */
  private async ensurePluginsPrefixExists(): Promise<void> {
    const backend = this.getStorageBackend()
    if (!backend) {
      return
    }

    try {
      const exists = await backend.exists('.keep')
      if (!exists) {
        await backend.write('.keep', Buffer.alloc(0))
        this.pluginLogger.info('Created plugins/.keep in S3')
      }
    } catch (error) {
      this.pluginLogger.error({ err: error }, 'Failed to create plugins/.keep in S3')
    }
  }

  /**
   * Recursively sync all files from a marketplace directory to S3.
   * Skips .git/ directories.
   */
  private async syncMarketplaceToS3(marketplaceDir: string, repoName: string): Promise<void> {
    const backend = this.getStorageBackend()
    if (!backend) {
      return
    }

    const files = await this.walkDirectory(marketplaceDir)
    this.pluginLogger.info({ repoName, fileCount: files.length }, 'Syncing marketplace to S3')

    for (const filePath of files) {
      const relativePath = path.relative(marketplaceDir, filePath)
      const s3Key = `marketplaces/${repoName}/${relativePath}`
      const content = await fs.readFile(filePath)
      await backend.write(s3Key, Buffer.from(content))
    }

    this.pluginLogger.info({ repoName, fileCount: files.length }, 'Marketplace synced to S3')
  }

  /**
   * Delete all S3 objects for a marketplace.
   */
  private async deleteMarketplaceFromS3(repoName: string): Promise<void> {
    const backend = this.getStorageBackend()
    if (!backend) {
      return
    }

    try {
      const keys = await backend.list(`marketplaces/${repoName}/`)
      for (const key of keys) {
        await backend.delete(key)
      }
      this.pluginLogger.info({ repoName, deletedCount: keys.length }, 'Marketplace deleted from S3')
    } catch (error) {
      this.pluginLogger.error({ err: error, repoName }, 'Failed to delete marketplace from S3')
    }
  }

  /**
   * Recursively walk a directory and return all file paths, skipping .git/.
   */
  private async walkDirectory(dir: string): Promise<string[]> {
    const files: string[] = []
    const entries = await fs.readdir(dir, { withFileTypes: true })

    for (const entry of entries) {
      if (entry.name === '.git') {
        continue
      }
      const fullPath = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        const nested = await this.walkDirectory(fullPath)
        files.push(...nested)
      } else {
        files.push(fullPath)
      }
    }

    return files
  }
}
