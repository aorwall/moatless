import type { GetFileResponse } from '@figma/rest-api-spec'
import {
  allExtractors,
  createFigmaClipboardParser,
  generateYamlFromExtractors,
  normalizeClipboardTree,
} from '@moatless/figma'
import type {
  FigmaOAuthTokenResponse,
  FigmaOAuthUrlResponse,
  FigmaPKCEVerifier,
  FigmaUserConfig,
  Node,
} from '@moatless/types'
import { inject, injectable } from 'inversify'
import { SessionContext } from '../features/auth/index'
import { StorageSymbol } from '../storage/storage-backend'
import { logger } from '../utils/logger'
import { UploadService } from './upload.service'

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
@injectable()
export class FigmaService {
  private readonly apiBaseUrl = 'https://api.figma.com'
  private readonly oauthClientId: string | undefined
  private readonly oauthClientSecret: string | undefined
  private readonly oauthRedirectUri: string | undefined

  constructor(
    @inject(StorageSymbol)
    private storage: {
      saveFigmaImport: (metadata: import('@moatless/types').FigmaImportMetadata) => Promise<void>
      getFigmaImport: (
        importId: string,
      ) => Promise<import('@moatless/types').FigmaImportMetadata | null>
      save: (key: string, data: unknown) => Promise<void>
      get: <T = unknown>(key: string) => Promise<T | null>
      delete: (key: string) => Promise<void>
    },
    @inject(UploadService) private upload: UploadService,
  ) {
    // OAuth configuration
    this.oauthClientId = process.env['FIGMA_OAUTH_CLIENT_ID']
    this.oauthClientSecret = process.env['FIGMA_OAUTH_CLIENT_SECRET']
    this.oauthRedirectUri = process.env['FIGMA_OAUTH_REDIRECT_URI']
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
    try {
      const accessToken = await this.getAccessToken()

      const url = `${this.apiBaseUrl}${endpoint}`
      logger.debug('Figma API request', { url })

      // Create abort controller for timeout
      const controller = new AbortController()
      const timeoutId = setTimeout(() => controller.abort(), 30000) // 30 second timeout

      const response = await fetch(url, {
        headers: {
          'X-Figma-Token': accessToken,
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
   * Downloads the node image, stores it, and creates metadata records
   * @param url - Figma URL with file key and node-id parameter
   * @param includeImageInPrompt - Whether to include the image in agent prompts
   * @returns Import result with IDs and metadata
   */
  async importNode(
    url: string,
    includeImageInPrompt = false,
  ): Promise<{
    importId: string
    fileId: string
    nodeId: string
    fileName: string
    nodeName: string
    createdAt: string
    hasImage: boolean
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

    // Always download and store image for API imports (for UI display)
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
      hasImage: true, // Always true for API imports - images are always downloaded
      includeImageInPrompt, // User preference for including image in agent context
    }

    // Save Figma metadata
    await this.storage.saveFigmaImport(figmaMetadata)

    logger.info('Figma node imported successfully', {
      importId,
      fileId: importId,
      nodeId,
      fileName: nodeData.fileName,
      nodeName: nodeData.node.name,
      hasImage: true,
      includeImageInPrompt,
    })

    // Return response
    return {
      importId,
      fileId: importId,
      nodeId,
      fileName: nodeData.fileName,
      nodeName: nodeData.node.name,
      createdAt: figmaMetadata.createdAt,
      hasImage: true,
    }
  }

  /**
   * Import Figma design from clipboard data
   * Parses fig-kiwie binary format, stores metadata with full node tree, and returns import details
   * @param clipboardHtml - Raw HTML clipboard content from Figma
   * @param _includeImageInPrompt - Ignored for clipboard imports (always false since no image exists)
   * @returns Import result with IDs and metadata
   */
  async importClipboard(
    clipboardHtml: string,
    _includeImageInPrompt = false,
  ): Promise<{
    importId: string
    fileId: string
    nodeId: string
    fileName: string
    nodeName: string
    createdAt: string
    hasImage: boolean
  }> {
    const { randomUUID } = await import('node:crypto')
    // Parse clipboard HTML to extract Figma design data
    const parser = createFigmaClipboardParser({ logger })
    const parseResult = parser.parseClipboardHTML(clipboardHtml)

    logger.info('Figma clipboard parsed', {
      formatType: parseResult.formatType,
      version: parseResult.version,
      nodeCount: parseResult.nodeCount,
      fileKey: parseResult.metadata.fileKey,
    })

    // Normalize clipboard tree to REST API-compatible format
    const normalized = normalizeClipboardTree(parseResult.root, { contextRoot: parseResult.root })

    logger.info('Figma clipboard normalized', {
      rootId: normalized.id,
      rootType: normalized.type,
      rootName: normalized.name,
    })

    // Generate UUID for import (same for both importId and fileId)
    const importId = randomUUID()

    // Create Figma import metadata with normalized node tree (REST API-compatible)
    const figmaMetadata: import('@moatless/types').FigmaImportMetadata = {
      importId,
      fileId: importId,
      url: undefined, // No URL for clipboard imports
      fileName: normalized.name,
      nodeName: normalized.name,
      nodeId: parseResult.metadata.pasteID.toString(),
      nodeMetadata: normalized as unknown as Record<string, unknown>,
      createdAt: new Date().toISOString(),
      hasImage: false, // Clipboard imports never have images
      includeImageInPrompt: false, // Always false for clipboard imports (no image available)
    }

    // Save Figma metadata
    await this.storage.saveFigmaImport(figmaMetadata)

    logger.info('Figma clipboard import saved successfully', {
      importId,
      fileId: importId,
      nodeId: figmaMetadata.nodeId,
      fileName: figmaMetadata.fileName,
      nodeName: figmaMetadata.nodeName,
      nodeCount: parseResult.nodeCount,
      hasImage: false,
    })

    // Return response
    return {
      importId,
      fileId: importId,
      nodeId: figmaMetadata.nodeId,
      fileName: figmaMetadata.fileName,
      nodeName: figmaMetadata.nodeName,
      createdAt: figmaMetadata.createdAt,
      hasImage: false,
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

    // Track absolute image path for inclusion in context when user preference is enabled
    let imagePath: string | null = null

    // Conditionally copy image to workspace based on user preference
    // This controls whether images are included in the agent's prompt/context
    if (figmaImport.includeImageInPrompt && figmaImport.hasImage) {
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

        // Store absolute path for inclusion in context
        imagePath = targetPath

        logger.info('Figma image copied to workspace for agent context', {
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
      logger.debug('Figma image not copied to workspace (user preference or no image available)', {
        importId,
        includeImageInPrompt: figmaImport.includeImageInPrompt,
        hasImage: figmaImport.hasImage,
      })
    }

    // Format context with YAML output using the extractor system
    // Both REST API and normalized clipboard imports are in the same format
    const normalizedNode = figmaImport.nodeMetadata as unknown as Node

    // Wrap the node in a GetFileResponse structure for the YAML generator
    const figmaResponse: GetFileResponse = {
      name: figmaImport.fileName,
      lastModified: figmaImport.createdAt,
      thumbnailUrl: '',
      version: '1.0',
      role: 'owner',
      editorType: 'figma',
      linkAccess: 'view',
      document: {
        id: '0:0',
        name: 'Document',
        type: 'DOCUMENT',
        scrollBehavior: 'SCROLLS',
        children: [normalizedNode],
      },
      components: {},
      componentSets: {},
      schemaVersion: 0,
      styles: {},
    } as unknown as GetFileResponse

    // Generate YAML using the extractor system
    const yamlContext = generateYamlFromExtractors(figmaResponse, {
      extractors: allExtractors,
    })

    // Include image path in context if images are enabled and image was successfully copied
    if (imagePath) {
      return `Figma Design: ${figmaImport.fileName} - ${figmaImport.nodeName}

INSTRUCTIONS:
- Reproduce this Figma component using the structured node data below
- The YAML data contains the complete component specification (layout, text, styles, hierarchy)
- An image is provided at: ${imagePath}
- Use the image as a visual reference when the node data is unclear or ambiguous
- Prioritize the structured node data as the source of truth

${yamlContext}`
    }

    return `Figma Design: ${figmaImport.fileName} - ${figmaImport.nodeName}

INSTRUCTIONS:
- Reproduce this Figma component using the structured node data below
- The YAML data contains the complete component specification (layout, text, styles, hierarchy)
- Use the node metadata, layout properties, and style definitions to build an accurate implementation

${yamlContext}`
  }

  /**
   * Generate PKCE code verifier and challenge
   * @see https://datatracker.ietf.org/doc/html/rfc7636
   */
  private async generatePKCE(): Promise<{ verifier: string; challenge: string }> {
    const { randomBytes, createHash } = await import('node:crypto')

    // Generate random verifier (43-128 characters)
    const verifier = randomBytes(32).toString('base64url')

    // Generate challenge using S256 method
    const challenge = createHash('sha256').update(verifier).digest('base64url')

    return { verifier, challenge }
  }

  /**
   * Generate Figma OAuth authorization URL with PKCE
   * @param userId - User ID to associate with this OAuth flow
   * @returns OAuth URL and state parameter
   */
  async generateOAuthUrl(userId: string): Promise<FigmaOAuthUrlResponse> {
    if (!(this.oauthClientId && this.oauthRedirectUri)) {
      throw new Error(
        'Figma OAuth not configured. Missing FIGMA_OAUTH_CLIENT_ID or FIGMA_OAUTH_REDIRECT_URI',
      )
    }

    const { randomBytes } = await import('node:crypto')

    // Generate PKCE verifier and challenge
    const { verifier, challenge } = await this.generatePKCE()

    // Generate random state for CSRF protection
    const state = randomBytes(32).toString('hex')

    // Store verifier temporarily (30 second expiration)
    const pkceData: FigmaPKCEVerifier = {
      verifier,
      state,
      userId,
      createdAt: new Date().toISOString(),
    }
    await this.storage.save(`figma-oauth-verifiers/${state}`, pkceData)

    // Build OAuth URL
    const params = new URLSearchParams({
      client_id: this.oauthClientId,
      redirect_uri: this.oauthRedirectUri,
      scope: 'current_user:read file_content:read file_metadata:read',
      state,
      response_type: 'code',
      code_challenge: challenge,
      code_challenge_method: 'S256',
    })

    const authUrl = `https://www.figma.com/oauth?${params.toString()}`

    logger.info('Generated Figma OAuth URL', { userId, state })

    return { authUrl, state }
  }

  /**
   * Exchange OAuth code for access token
   * @param code - Authorization code from Figma
   * @param state - State parameter from OAuth callback
   * @param userId - User ID making the request
   * @returns void (token is saved to storage)
   */
  async exchangeOAuthToken(code: string, state: string, userId: string): Promise<void> {
    if (!(this.oauthClientId && this.oauthClientSecret && this.oauthRedirectUri)) {
      throw new Error('Figma OAuth not configured')
    }

    // Retrieve and validate PKCE verifier
    const pkceData = await this.storage.get<FigmaPKCEVerifier>(`figma-oauth-verifiers/${state}`)
    if (!pkceData) {
      throw new Error('Invalid or expired OAuth state')
    }

    // Validate state matches and user matches
    if (pkceData.state !== state || pkceData.userId !== userId) {
      throw new Error('OAuth state mismatch')
    }

    // Check expiration (30 seconds)
    const createdAt = new Date(pkceData.createdAt).getTime()
    const now = Date.now()
    if (now - createdAt > 30000) {
      await this.storage.delete(`figma-oauth-verifiers/${state}`)
      throw new Error('OAuth code expired')
    }

    // Exchange code for token
    const tokenParams = new URLSearchParams({
      client_id: this.oauthClientId,
      client_secret: this.oauthClientSecret,
      code,
      redirect_uri: this.oauthRedirectUri,
      grant_type: 'authorization_code',
      code_verifier: pkceData.verifier,
    })

    try {
      const response = await fetch('https://api.figma.com/v1/oauth/token', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/x-www-form-urlencoded',
        },
        body: tokenParams.toString(),
      })

      if (!response.ok) {
        const errorText = await response.text()
        logger.error('Figma OAuth token exchange failed', new Error(`Status: ${response.status}`), {
          status: response.status,
          error: errorText,
        })
        throw new Error('Failed to exchange OAuth code for token')
      }

      const tokenResponse = (await response.json()) as FigmaOAuthTokenResponse

      // Save user token
      const expiresAt = new Date(Date.now() + tokenResponse.expires_in * 1000).toISOString()
      const userConfig: FigmaUserConfig = {
        userId,
        authMethod: 'oauth',
        accessToken: tokenResponse.access_token,
        refreshToken: tokenResponse.refresh_token,
        expiresAt,
        connectedAt: new Date().toISOString(),
        figmaUserId: tokenResponse.user_id_string,
      }

      await this.storage.save(`users/${userId}/figma`, userConfig)

      // Clean up verifier
      await this.storage.delete(`figma-oauth-verifiers/${state}`)

      logger.info('Figma OAuth token saved successfully', {
        userId,
        figmaUserId: tokenResponse.user_id_string,
      })
    } catch (error) {
      // Clean up verifier on error
      await this.storage.delete(`figma-oauth-verifiers/${state}`)
      throw error
    }
  }

  /**
   * Get user's Figma token from storage
   * @param userId - User ID
   * @returns User's Figma configuration or null
   */
  async getUserToken(userId: string): Promise<FigmaUserConfig | null> {
    return await this.storage.get<FigmaUserConfig>(`users/${userId}/figma`)
  }

  /**
   * Delete user's Figma token from storage
   * @param userId - User ID
   */
  async deleteUserToken(userId: string): Promise<void> {
    await this.storage.delete(`users/${userId}/figma`)
    logger.info('Figma token deleted', { userId })
  }

  /**
   * Save Personal Access Token for user
   * @param userId - User ID
   * @param accessToken - Figma Personal Access Token
   */
  async savePersonalAccessToken(userId: string, accessToken: string): Promise<void> {
    const userConfig: FigmaUserConfig = {
      userId,
      authMethod: 'pat',
      accessToken,
      connectedAt: new Date().toISOString(),
    }

    await this.storage.save(`users/${userId}/figma`, userConfig)

    logger.info('Figma Personal Access Token saved successfully', {
      userId,
      authMethod: 'pat',
    })
  }

  /**
   * Get access token for API requests
   * Uses user-specific token (OAuth or PAT) from session context
   * @returns Access token
   * @throws Error if user is not authenticated or Figma is not connected
   */
  private async getAccessToken(): Promise<string> {
    const userId = SessionContext.getCurrentUserId()
    if (!userId) {
      throw new Error('User not authenticated')
    }

    const userConfig = await this.getUserToken(userId)
    if (!userConfig) {
      throw new Error('Figma not connected. Please connect your Figma account in settings.')
    }

    // TODO: Handle token refresh if expired (OAuth only)
    return userConfig.accessToken
  }

  /**
   * Check if OAuth is enabled/configured on the server
   * @returns true if FIGMA_OAUTH_CLIENT_ID is set
   */
  isOAuthEnabled(): boolean {
    return !!this.oauthClientId
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
