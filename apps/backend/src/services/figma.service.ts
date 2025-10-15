import type { Node } from '@moatless/types'
import { generateFigmaContextText } from '../utils/figma-transformers.js'
import { logger } from '../utils/logger.js'

/**
 * Response from getting a single node from Figma API
 */
interface FigmaNodeResponse {
  name: string
  role: string
  lastModified: string
  nodes: Record<string, { document: Node } | null>
}

/**
 * Response from getting a rendered image
 */
interface FigmaNodeImageResponse {
  err: string | null
  images: Record<string, string | null>
}

/**
 * Options for getting node data
 */
export interface GetNodeOptions {
  version?: string
  depth?: number
  geometry?: 'paths' | 'none'
}

/**
 * Options for getting node images
 */
export interface GetNodeImageOptions {
  scale?: number
  format?: 'jpg' | 'png' | 'svg' | 'pdf'
}

/**
 * Service for interacting with Figma's REST API
 * Focused on fetching individual nodes and their rendered images
 * @see https://developers.figma.com/docs/rest-api/file-endpoints
 */
export class FigmaService {
  private readonly apiBaseUrl = 'https://api.figma.com'
  private readonly accessToken: string | undefined
  private readonly imagesEnabled: boolean

  constructor(
    private storage: {
      saveFigmaImport: (metadata: import('@moatless/types').FigmaImportMetadata) => Promise<void>
      getFigmaImport: (
        importId: string,
      ) => Promise<import('@moatless/types').FigmaImportMetadata | null>
    },
    private upload: {
      uploadFile: (buffer: Buffer, filename: string, mimeType: string) => Promise<unknown>
      getFile: (fileId: string) => Promise<{ buffer: Buffer; metadata: { filename: string } }>
    },
  ) {
    this.accessToken = process.env['FIGMA_ACCESS_TOKEN']
    if (!this.accessToken) {
      logger.warn('FIGMA_ACCESS_TOKEN not set in environment variables')
    }

    // Feature flag for image download and storage (disabled by default)
    this.imagesEnabled = process.env['FIGMA_IMAGES_ENABLED'] === 'true'
    if (!this.imagesEnabled) {
      logger.info(
        'Figma image download is disabled. Only text-based design metadata will be stored.',
      )
    }
  }

  /**
   * Build query string from options object
   */
  private buildQueryString(params: Record<string, unknown>): string {
    const query = new URLSearchParams()
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined && value !== null) {
        query.append(key, String(value))
      }
    }
    const queryString = query.toString()
    return queryString ? `?${queryString}` : ''
  }

  /**
   * Make a request to the Figma API
   */
  private async request<T>(endpoint: string): Promise<T | null> {
    if (!this.accessToken) {
      logger.error('Figma API: Access token not configured')
      return null
    }

    try {
      const url = `${this.apiBaseUrl}${endpoint}`
      logger.debug('Figma API request', { url })

      // Create abort controller for timeout
      const controller = new AbortController()
      const timeoutId = setTimeout(() => controller.abort(), 30000) // 30 second timeout

      const response = await fetch(url, {
        headers: {
          'X-Figma-Token': this.accessToken,
        },
        signal: controller.signal,
      })

      clearTimeout(timeoutId)

      if (!response.ok) {
        const errorText = await response.text()
        logger.error('Figma API: Request failed', new Error(`Status: ${response.status}`), {
          status: response.status,
          statusText: response.statusText,
          error: errorText,
        })
        return null
      }

      return (await response.json()) as T
    } catch (error) {
      if ((error as Error).name === 'AbortError') {
        logger.error('Figma API: Request timeout after 30 seconds')
        return null
      }
      logger.error('Error making Figma API request', error as Error)
      return null
    }
  }

  /**
   * Get a single node by ID from Figma API
   * @param fileKey - The file key from the Figma URL
   * @param nodeId - Node ID to retrieve
   * @param options - Optional parameters for the request
   * @returns Node data directly from Figma API, or null if not found
   * @example
   * const result = await figma.getNode('Z6ij6B0g9en70YQ9sfLUQB', '3376:185062')
   * if (result) {
   *   console.log(result.node.name)    // Node name
   *   console.log(result.fileName)     // File name
   * }
   */
  async getNode(
    fileKey: string,
    nodeId: string,
    options?: GetNodeOptions,
  ): Promise<{ node: Node; fileName: string } | null> {
    const queryParams = {
      ids: nodeId,
      ...options,
    }

    const queryString = this.buildQueryString(queryParams)
    const result = await this.request<FigmaNodeResponse>(`/v1/files/${fileKey}/nodes${queryString}`)

    const nodeData = result?.nodes[nodeId]
    if (!nodeData) {
      return null
    }

    return {
      node: nodeData.document,
      fileName: result.name,
    }
  }

  /**
   * Get rendered image URL for a single node
   * @param fileKey - The file key from the Figma URL
   * @param nodeId - Node ID to render
   * @param options - Optional parameters for rendering
   * @returns Image URL or null if failed
   * @example
   * const imageUrl = await figma.getNodeImage('Z6ij6B0g9en70YQ9sfLUQB', '3376-185062', {
   *   format: 'png',
   *   scale: 2
   * })
   */
  async getNodeImage(
    fileKey: string,
    nodeId: string,
    options?: GetNodeImageOptions,
  ): Promise<string | null> {
    const queryParams = {
      ids: nodeId,
      format: options?.format || 'png',
      scale: options?.scale || 1,
    }

    const queryString = this.buildQueryString(queryParams)
    const result = await this.request<FigmaNodeImageResponse>(`/v1/images/${fileKey}${queryString}`)

    if (!result || result.err) {
      logger.error('Figma API: Image rendering failed', new Error(result?.err || 'Unknown error'))
      return null
    }

    return result.images[nodeId] ?? null
  }

  /**
   * Import a Figma node by URL
   * Downloads the node image (if images enabled), stores it, and creates metadata records
   * @param url - Figma URL with file key and node-id parameter
   * @returns Import result with IDs and metadata
   */
  async importNode(url: string): Promise<{
    importId: string
    fileId: string
    nodeId: string
    fileName: string
    nodeName: string
    createdAt: string
  }> {
    const { randomUUID } = await import('node:crypto')

    // Parse Figma URL to extract fileKey and nodeId
    const fileKey = FigmaService.extractFileKey(url)
    const nodeId = FigmaService.extractNodeId(url)

    if (!(fileKey && nodeId)) {
      throw new Error('Invalid Figma URL. Must include file key and node-id parameter.')
    }

    // Fetch node metadata (always required)
    const nodeData = await this.getNode(fileKey, nodeId)
    if (!nodeData) {
      throw new Error('Failed to fetch node data from Figma API')
    }

    // Generate UUID for import (same for both importId and fileId)
    const importId = randomUUID()

    // Conditionally download and store image based on feature flag
    if (this.imagesEnabled) {
      const promises = await import('node:fs/promises')
      const path = await import('node:path')

      // Fetch node image URL
      const imageUrl = await this.getNodeImage(fileKey, nodeId, {
        format: 'png',
        scale: 1,
      })

      if (!imageUrl) {
        throw new Error('Failed to fetch image URL from Figma API')
      }

      // Download image from Figma CDN
      const imageResponse = await fetch(imageUrl)
      if (!imageResponse.ok) {
        throw new Error(`Failed to download image from Figma CDN: ${imageResponse.statusText}`)
      }

      const imageBuffer = Buffer.from(await imageResponse.arrayBuffer())

      // Save image using upload service
      const filename = `${nodeData.node.name.replace(/[^a-z0-9]/gi, '-')}.png`
      await this.upload.uploadFile(imageBuffer, filename, 'image/png')

      // Get upload directories - use environment variable with fallback
      const { getDataDir } = await import('../utils/data-dir.js')
      const dataDir = getDataDir()
      const uploadDir = process.env['UPLOAD_DIR'] || path.join(dataDir, 'uploads')
      const metadataDir = path.join(uploadDir, '.metadata')

      // Create directories
      await promises.mkdir(uploadDir, { recursive: true })
      await promises.mkdir(metadataDir, { recursive: true })

      // Store file with our UUID
      const filePath = path.join(uploadDir, importId)
      await promises.writeFile(filePath, imageBuffer)

      // Create upload metadata
      const uploadMetadata = {
        fileId: importId,
        filename,
        mimeType: 'image/png',
        size: imageBuffer.length,
        uploadedAt: new Date().toISOString(),
      }

      // Store upload metadata
      const metadataPath = path.join(metadataDir, `${importId}.json`)
      await promises.writeFile(metadataPath, JSON.stringify(uploadMetadata, null, 2))

      logger.debug('Figma image downloaded and stored', {
        importId,
        filename,
      })
    } else {
      logger.debug('Figma image download skipped (images disabled)', {
        importId,
      })
    }

    // Create Figma import metadata (always required)
    const figmaMetadata: import('@moatless/types').FigmaImportMetadata = {
      importId,
      fileId: importId,
      url,
      fileName: nodeData.fileName,
      nodeName: nodeData.node.name,
      nodeId,
      nodeMetadata: nodeData.node as unknown as Record<string, unknown>,
      createdAt: new Date().toISOString(),
    }

    // Save Figma metadata
    await this.storage.saveFigmaImport(figmaMetadata)

    logger.info('Figma node imported successfully', {
      importId,
      fileId: importId,
      nodeId,
      fileName: nodeData.fileName,
      nodeName: nodeData.node.name,
      imagesEnabled: this.imagesEnabled,
    })

    // Return response
    return {
      importId,
      fileId: importId,
      nodeId,
      fileName: nodeData.fileName,
      nodeName: nodeData.node.name,
      createdAt: figmaMetadata.createdAt,
    }
  }

  /**
   * Get Figma context for workspace
   * Fetches Figma import, copies image to workspace (if images enabled), and returns formatted context text
   * @param importId - The Figma import ID
   * @param workspacePath - Path to the workspace
   * @returns Formatted context text for the Figma design
   */
  async getFigmaContextForWorkspace(importId: string, workspacePath: string): Promise<string> {
    // Fetch Figma import metadata from storage
    const figmaImport = await this.storage.getFigmaImport(importId)
    if (!figmaImport) {
      throw new Error(`Figma import not found: ${importId}`)
    }

    // Conditionally copy image to workspace based on feature flag
    if (this.imagesEnabled) {
      const promises = await import('node:fs/promises')
      const path = await import('node:path')

      try {
        // Get the Figma image file from upload service
        const { buffer, metadata } = await this.upload.getFile(figmaImport.fileId)

        // Copy to workspace uploads directory
        const uploadsDir = path.join(workspacePath, 'uploads')
        await promises.mkdir(uploadsDir, { recursive: true })

        const targetPath = path.join(uploadsDir, metadata.filename)
        await promises.writeFile(targetPath, buffer)

        logger.info('Figma image copied to workspace', {
          importId,
          filename: metadata.filename,
          targetPath,
        })
      } catch (error) {
        // Log warning but don't fail - text context can still be useful
        logger.warn('Failed to copy Figma image to workspace (continuing with text-only context)', {
          importId,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    } else {
      logger.debug('Figma image copy skipped (images disabled)', {
        importId,
      })
    }

    // Format context with readable design description (always return text context)
    return generateFigmaContextText(
      figmaImport.nodeMetadata,
      figmaImport.fileName,
      figmaImport.nodeName,
    )
  }

  /**
   * Extract file key from a Figma URL
   * @param url - Figma file URL
   * @returns File key or null if URL is invalid
   * @example
   * extractFileKey('https://www.figma.com/design/Z6ij6B0g9en70YQ9sfLUQB/Tech-FP---Main')
   * // Returns: 'Z6ij6B0g9en70YQ9sfLUQB'
   */
  static extractFileKey(url: string): string | null {
    try {
      const urlObj = new URL(url)
      const pathParts = urlObj.pathname.split('/')

      // Figma URLs have format: /file/:key or /design/:key
      const fileKeyIndex = pathParts.findIndex((part) => part === 'file' || part === 'design')
      if (fileKeyIndex !== -1 && pathParts[fileKeyIndex + 1]) {
        return pathParts[fileKeyIndex + 1] ?? null
      }

      return null
    } catch (error) {
      logger.error('Error parsing Figma URL', error as Error)
      return null
    }
  }

  /**
   * Extract node ID from a Figma URL
   * @param url - Figma file URL with node-id parameter
   * @returns Node ID in API format (with colon) or null if not found
   * @example
   * extractNodeId('https://www.figma.com/design/xyz?node-id=3376-185062')
   * // Returns: '3376:185062' (note: dash converted to colon for API)
   */
  static extractNodeId(url: string): string | null {
    try {
      const urlObj = new URL(url)
      const nodeId = urlObj.searchParams.get('node-id')
      if (!nodeId) {
        return null
      }
      // Convert dash to colon for API format
      // URLs use format: 3376-185062
      // API expects format: 3376:185062
      return nodeId.replace(/-/g, ':')
    } catch (error) {
      logger.error('Error parsing Figma URL for node ID', error as Error)
      return null
    }
  }
}

/**
 * Create a new FigmaService instance with dependencies
 */
export function createFigmaService(
  storage: {
    saveFigmaImport: (metadata: import('@moatless/types').FigmaImportMetadata) => Promise<void>
    getFigmaImport: (
      importId: string,
    ) => Promise<import('@moatless/types').FigmaImportMetadata | null>
  },
  upload: {
    uploadFile: (buffer: Buffer, filename: string, mimeType: string) => Promise<unknown>
    getFile: (fileId: string) => Promise<{ buffer: Buffer; metadata: { filename: string } }>
  },
): FigmaService {
  return new FigmaService(storage, upload)
}
