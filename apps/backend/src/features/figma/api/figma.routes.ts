import type { GetFileResponse } from '@figma/rest-api-spec'
import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  deleteFigmaConfigRoute,
  exchangeFigmaTokenRoute,
  getFigmaConfigRoute,
  getFigmaImportImageRoute,
  getFigmaImportRoute,
  getFigmaOAuthUrlRoute,
  importFigmaClipboardRoute,
  importFigmaNodeRoute,
  saveFigmaPATRoute,
} from '@moatless/api-contract/figma'
import { allExtractors, generateYamlFromExtractors } from '@moatless/figma'
import type { Node } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { logger } from '../../../utils/logger'
import { SessionContext } from '../../auth/index'
import { FileStorageService } from '../../file-storage'
import { FigmaService } from '../services/figma.service'
import { FigmaStorage } from '../storage/figma.storage'

export function registerFigmaRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/figma/import
   * Import a Figma node by URL
   */
  app.openapi(importFigmaNodeRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)
      const { url, includeImageInPrompt = false } = c.req.valid('json')

      // Import the node using the service
      const result = await figmaService.importNode(url, includeImageInPrompt)

      // Return response
      return c.json(result)
    } catch (error) {
      logger.error('Failed to import Figma node', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(500, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to import Figma node' })
    }
  })

  /**
   * POST /api/v1/figma/import-clipboard
   * Import Figma clipboard data
   */
  app.openapi(importFigmaClipboardRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)
      const { clipboardHtml, includeImageInPrompt = false } = c.req.valid('json')

      // Import clipboard using the service
      // Note: includeImageInPrompt is always false for clipboard imports (no image available)
      const result = await figmaService.importClipboard(clipboardHtml, includeImageInPrompt)

      // Return response
      return c.json(result)
    } catch (error) {
      logger.error('Failed to import Figma clipboard', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(400, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to import clipboard data' })
    }
  })

  /**
   * GET /api/v1/figma/imports/{importId}
   * Get Figma import metadata
   */
  app.openapi(getFigmaImportRoute, async (c) => {
    const { importId } = c.req.valid('param')

    try {
      const storage = appContainer.get(FigmaStorage)

      // Get Figma import metadata
      const metadata = await storage.getFigmaImport(importId)

      if (!metadata) {
        throw new HTTPException(404, { message: 'Import not found' })
      }

      // Generate YAML formatted prompt for AI context
      const normalizedNode = metadata.nodeMetadata as unknown as Node

      // Wrap the node in a GetFileResponse structure for the YAML generator
      const figmaResponse: GetFileResponse = {
        name: metadata.fileName,
        lastModified: metadata.createdAt,
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

      const prompt = generateYamlFromExtractors(figmaResponse, {
        extractors: allExtractors,
      })

      return c.json({
        ...metadata,
        prompt,
      })
    } catch (error) {
      logger.error('Failed to get Figma import', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to retrieve import metadata' })
    }
  })

  /**
   * GET /api/v1/figma/imports/{importId}/image
   * Get Figma import image
   */
  app.openapi(getFigmaImportImageRoute, async (c) => {
    const { importId } = c.req.valid('param')

    try {
      const storage = appContainer.get(FigmaStorage)
      const uploadService = appContainer.get(FileStorageService)

      // Get Figma import metadata to find fileId
      const metadata = await storage.getFigmaImport(importId)

      if (!metadata) {
        throw new HTTPException(404, { message: 'Import not found' })
      }

      // Check if this import has an image
      if (!metadata.hasImage) {
        throw new HTTPException(404, {
          message: 'No image available for this import (clipboard imports do not include images)',
        })
      }

      // Get file from upload service
      const { buffer, metadata: uploadMetadata } = await uploadService.getFile(metadata.fileId)

      // Return image with proper headers
      const headers: Record<string, string> = {
        'Content-Type': uploadMetadata.mimeType,
        'Cache-Control': 'public, max-age=31536000', // Cache for 1 year since images don't change
      }

      return new Response(buffer, { headers })
    } catch (error) {
      logger.error('Failed to get Figma import image', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error && error.message.includes('not found')) {
        throw new HTTPException(404, { message: 'Image not found' })
      }
      throw new HTTPException(500, { message: 'Failed to retrieve image' })
    }
  })

  /**
   * GET /api/v1/figma/oauth/url
   * Get Figma OAuth authorization URL
   */
  app.openapi(getFigmaOAuthUrlRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      // Generate OAuth URL with PKCE
      const result = await figmaService.generateOAuthUrl(userId)

      return c.json(result)
    } catch (error) {
      logger.error('Failed to generate Figma OAuth URL', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(500, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to generate OAuth URL' })
    }
  })

  /**
   * POST /api/v1/figma/oauth/token
   * Exchange OAuth code for access token
   */
  app.openapi(exchangeFigmaTokenRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const { code, state } = c.req.valid('json')

      // Exchange code for token
      await figmaService.exchangeOAuthToken(code, state, userId)

      return c.json({
        success: true,
        message: 'Figma account connected successfully',
      })
    } catch (error) {
      logger.error('Failed to exchange Figma OAuth token', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(400, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to exchange OAuth token' })
    }
  })

  /**
   * GET /api/v1/figma/config
   * Get user's Figma configuration
   */
  app.openapi(getFigmaConfigRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      // Check if OAuth is enabled on the server
      const oauthEnabled = figmaService.isOAuthEnabled()

      // Get user's Figma token
      const userConfig = await figmaService.getUserToken(userId)

      if (!userConfig) {
        return c.json({
          connected: false,
          oauthEnabled,
        })
      }

      return c.json({
        connected: true,
        oauthEnabled,
        authMethod: userConfig.authMethod,
        figmaUserId: userConfig.figmaUserId,
        connectedAt: userConfig.connectedAt,
        expiresAt: userConfig.expiresAt,
      })
    } catch (error) {
      logger.error('Failed to get Figma configuration', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to retrieve configuration' })
    }
  })

  /**
   * DELETE /api/v1/figma/config
   * Disconnect Figma OAuth
   */
  app.openapi(deleteFigmaConfigRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      // Delete user's Figma token
      await figmaService.deleteUserToken(userId)

      return c.json({
        success: true,
        message: 'Figma account disconnected successfully',
      })
    } catch (error) {
      logger.error('Failed to disconnect Figma', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to disconnect Figma' })
    }
  })

  /**
   * PUT /api/v1/figma/config/token
   * Save Personal Access Token
   */
  app.openapi(saveFigmaPATRoute, async (c) => {
    try {
      const figmaService = appContainer.get(FigmaService)

      // Get current user from session
      const userId = SessionContext.getCurrentUserId()
      if (!userId) {
        throw new HTTPException(401, { message: 'Not authenticated' })
      }

      const { accessToken } = c.req.valid('json')

      // Save Personal Access Token
      await figmaService.savePersonalAccessToken(userId, accessToken)

      return c.json({
        success: true,
        message: 'Figma Personal Access Token saved successfully',
      })
    } catch (error) {
      logger.error('Failed to save Figma Personal Access Token', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(400, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to save Personal Access Token' })
    }
  })
}
