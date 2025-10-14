import type { ParsedFigmaNode, RawFigmaNode } from '@moatless/types'
import { logger } from '../utils/logger.js'
import { parseNode } from './figma-parser.js'

/**
 * Response from getting a single node
 */
interface FigmaNodeResponse {
  name: string
  role: string
  lastModified: string
  nodes: Record<string, { document: RawFigmaNode } | null>
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

  constructor() {
    this.accessToken = process.env['FIGMA_ACCESS_TOKEN']
    if (!this.accessToken) {
      logger.warn('FIGMA_ACCESS_TOKEN not set in environment variables')
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
   * Get a single node by ID
   * @param fileKey - The file key from the Figma URL
   * @param nodeId - Node ID to retrieve
   * @param options - Optional parameters for the request
   * @returns Parsed node data with both simplified and raw formats, or null if not found
   * @example
   * const result = await figma.getNode('Z6ij6B0g9en70YQ9sfLUQB', '3376:185062')
   * if (result) {
   *   console.log(result.simplified.name) // Simplified, typed data
   *   console.log(result.raw.id)          // Raw API response
   * }
   */
  async getNode(
    fileKey: string,
    nodeId: string,
    options?: GetNodeOptions,
  ): Promise<ParsedFigmaNode | null> {
    const queryParams = {
      ids: nodeId,
      ...options,
    }

    const queryString = this.buildQueryString(queryParams)
    const result = await this.request<FigmaNodeResponse>(`/v1/files/${fileKey}/nodes${queryString}`)

    const node = result?.nodes[nodeId]
    if (!node) {
      return null
    }

    // Parse the raw node into simplified format
    const simplified = parseNode(node.document)

    return {
      simplified,
      raw: node.document,
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

// Singleton instance
let figmaServiceInstance: FigmaService | null = null

/**
 * Get the singleton FigmaService instance
 */
export function getFigmaService(): FigmaService {
  if (!figmaServiceInstance) {
    figmaServiceInstance = new FigmaService()
  }
  return figmaServiceInstance
}

/**
 * Reset the singleton instance (useful for testing)
 */
export function resetFigmaServiceInstance(): void {
  figmaServiceInstance = null
}
