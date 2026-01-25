import { exec } from 'node:child_process'
import { existsSync, promises as fs } from 'node:fs'
import * as path from 'node:path'
import { promisify } from 'node:util'
import type { MarketplaceManifest, PluginInfo, PluginMarketplace } from '@moatless/types'
import { MarketplaceManifestSchema } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { v4 as uuidv4 } from 'uuid'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
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

  constructor(
    @inject(PluginMarketplaceRepository) private readonly repository: PluginMarketplaceRepository,
  ) {
    const dataDir = getDataDir()
    this.pluginsDir = process.env['PLUGINS_DIR'] || path.join(dataDir, 'plugins')
    this.marketplacesDir = path.join(this.pluginsDir, 'marketplaces')
    this.sandboxPluginsDir = process.env['SANDBOX_PLUGINS_DIR'] || '/opt/moatless/plugins'
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
    await this.cloneMarketplace(normalizedUrl, marketplaceDir)

    // Read and validate marketplace.json
    const manifest = await this.readMarketplaceManifest(marketplaceDir)

    // Create marketplace record
    const marketplace: PluginMarketplace = {
      id: uuidv4(),
      name: manifest.name || repoName,
      repoUrl: normalizedUrl,
      ownerName: manifest.owner?.name,
      description: manifest.metadata?.description,
      lastSyncedAt: new Date(),
      createdAt: new Date(),
      updatedAt: new Date(),
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

    const marketplaceDir = path.join(
      this.marketplacesDir,
      this.extractRepoName(marketplace.repoUrl),
    )

    if (!existsSync(marketplaceDir)) {
      // Directory was deleted - re-clone
      this.pluginLogger.info({ id }, 'Marketplace directory missing, re-cloning')
      await this.cloneMarketplace(marketplace.repoUrl, marketplaceDir)
    } else {
      // Pull latest changes
      this.pluginLogger.info({ id }, 'Pulling latest changes for marketplace')
      try {
        await execAsync('git fetch origin && git reset --hard origin/HEAD', {
          cwd: marketplaceDir,
          timeout: 60000, // 60 seconds timeout
        })
      } catch (error) {
        this.pluginLogger.error({ err: error, id }, 'Failed to sync marketplace')
        throw new Error(
          `Failed to sync marketplace: ${error instanceof Error ? error.message : String(error)}`,
        )
      }
    }

    // Re-read manifest to update metadata
    const manifest = await this.readMarketplaceManifest(marketplaceDir)

    // Update marketplace record
    const updatedMarketplace: PluginMarketplace = {
      ...marketplace,
      ownerName: manifest.owner?.name,
      description: manifest.metadata?.description,
      lastSyncedAt: new Date(),
      updatedAt: new Date(),
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
      const marketplaceDir = path.join(
        this.marketplacesDir,
        this.extractRepoName(marketplace.repoUrl),
      )

      if (!existsSync(marketplaceDir)) {
        this.pluginLogger.warn(
          { marketplaceId: marketplace.id },
          'Marketplace directory missing, skipping',
        )
        continue
      }

      try {
        const manifest = await this.readMarketplaceManifest(marketplaceDir)

        for (const pluginEntry of manifest.plugins) {
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
      const manifest = await this.readMarketplaceManifest(marketplaceDir)
      const pluginEntry = manifest.plugins.find((p) => p.name === pluginName)

      if (pluginEntry?.source) {
        // Use source field from manifest
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
    const marketplaceDir = path.join(
      this.marketplacesDir,
      this.extractRepoName(marketplace.repoUrl),
    )
    if (existsSync(marketplaceDir)) {
      await fs.rm(marketplaceDir, { recursive: true, force: true })
    }

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
      // Clone with sparse checkout for efficiency
      // Only download .claude-plugin/ and plugins/ directories
      await execAsync(`git clone --filter=blob:none --sparse "${repoUrl}" "${targetDir}"`, {
        timeout: 120000, // 2 minutes timeout
      })

      // Set up sparse checkout to only include relevant directories
      await execAsync('git sparse-checkout set .claude-plugin plugins', {
        cwd: targetDir,
        timeout: 30000,
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
}
