import { createWriteStream, promises as fs } from 'node:fs'
import path from 'node:path'
import archiver from 'archiver'
import { logger } from '../utils/logger.js'

interface NetlifyDeployResponse {
  success: boolean
  siteId?: string
  siteUrl?: string
  deployUrl?: string
  deployId?: string
  logs: string
  duration: number
}

interface NetlifySite {
  id: string
  name: string
  url: string
  ssl_url: string
}

interface NetlifyDeployResult {
  id: string
  url: string
  deploy_url: string
  site_id: string
}

/**
 * Service for deploying to Netlify using the API instead of CLI
 */
export class NetlifyService {
  private readonly baseUrl = 'https://api.netlify.com/api/v1'

  constructor() {}

  /**
   * Deploy a directory to Netlify using the API
   */
  async deployToNetlify(
    outputPath: string,
    siteName: string,
    authToken: string,
    existingSiteId?: string,
  ): Promise<NetlifyDeployResponse> {
    const startTime = Date.now()
    const logs: string[] = []

    try {
      logger.info('Starting Netlify API deployment', {
        outputPath,
        siteName,
        existingSiteId,
      })

      // Step 1: Verify output directory exists
      const outputExists = await this.pathExists(outputPath)
      if (!outputExists) {
        throw new Error(`Build output directory does not exist: ${outputPath}`)
      }

      // Step 2: Create ZIP file of the output directory
      const zipPath = path.join(path.dirname(outputPath), `${path.basename(outputPath)}.zip`)
      logs.push(`Creating ZIP file: ${zipPath}`)
      await this.createZipFile(outputPath, zipPath)

      // Step 3: Get or create site
      let siteId = existingSiteId
      let siteUrl: string

      if (!siteId) {
        logs.push('Creating new Netlify site...')
        const site = await this.createSite(siteName, authToken)
        siteId = site.id
        siteUrl = site.ssl_url
        logs.push(`Created site: ${site.name} (${siteId})`)
      } else {
        logs.push(`Using existing site: ${siteId}`)
        const site = await this.getSite(siteId, authToken)
        siteUrl = site.ssl_url
      }

      // Step 4: Deploy the ZIP file
      logs.push('Deploying ZIP file to Netlify...')
      const deployment = await this.deployZipFile(zipPath, siteId, authToken)
      logs.push(`Deployment completed: ${deployment.deploy_url}`)

      // Step 5: Cleanup ZIP file
      await fs.unlink(zipPath)
      logs.push('Cleaned up temporary ZIP file')

      const duration = Date.now() - startTime

      logger.info('Netlify deployment completed successfully', {
        siteId,
        siteUrl,
        deployUrl: deployment.deploy_url,
        duration,
      })

      return {
        success: true,
        siteId,
        siteUrl,
        deployUrl: deployment.deploy_url,
        deployId: deployment.id,
        logs: logs.join('\n'),
        duration,
      }
    } catch (error) {
      const duration = Date.now() - startTime
      const errorMessage = error instanceof Error ? error.message : String(error)
      logs.push(`Error: ${errorMessage}`)

      logger.error('Netlify deployment failed', error as Error, {
        outputPath,
        siteName,
        existingSiteId,
        duration,
      })

      return {
        success: false,
        logs: logs.join('\n'),
        duration,
      }
    }
  }

  /**
   * Create a new Netlify site
   */
  private async createSite(siteName: string, authToken: string): Promise<NetlifySite> {
    const response = await fetch(`${this.baseUrl}/sites`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${authToken}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        name: siteName,
      }),
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to create site: ${response.status} ${errorText}`)
    }

    return (await response.json()) as NetlifySite
  }

  /**
   * Get site information
   */
  private async getSite(siteId: string, authToken: string): Promise<NetlifySite> {
    const response = await fetch(`${this.baseUrl}/sites/${siteId}`, {
      method: 'GET',
      headers: {
        Authorization: `Bearer ${authToken}`,
      },
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to get site: ${response.status} ${errorText}`)
    }

    return (await response.json()) as NetlifySite
  }

  /**
   * Deploy a ZIP file to a Netlify site
   */
  private async deployZipFile(
    zipPath: string,
    siteId: string,
    authToken: string,
  ): Promise<NetlifyDeployResult> {
    const zipBuffer = await fs.readFile(zipPath)

    const response = await fetch(`${this.baseUrl}/sites/${siteId}/deploys`, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${authToken}`,
        'Content-Type': 'application/zip',
      },
      body: zipBuffer,
    })

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`Failed to deploy: ${response.status} ${errorText}`)
    }

    return (await response.json()) as NetlifyDeployResult
  }

  /**
   * Create a ZIP file from a directory
   */
  private async createZipFile(sourceDir: string, outputPath: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const output = createWriteStream(outputPath)
      const archive = archiver('zip', {
        zlib: { level: 9 }, // Maximum compression
      })

      output.on('close', () => {
        logger.debug(`ZIP file created: ${outputPath} (${archive.pointer()} bytes)`)
        resolve()
      })

      archive.on('error', (err) => {
        reject(err)
      })

      archive.pipe(output)

      // Add all files from the source directory
      archive.directory(sourceDir, false)

      // Finalize the archive
      archive.finalize()
    })
  }

  /**
   * Check if a path exists
   */
  private async pathExists(path: string): Promise<boolean> {
    try {
      await fs.access(path)
      return true
    } catch {
      return false
    }
  }
}

/**
 * Create a new NetlifyService instance
 */
export function createNetlifyService(): NetlifyService {
  return new NetlifyService()
}
