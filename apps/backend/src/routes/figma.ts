import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getFigmaImportImageRoute,
  getFigmaImportRoute,
  importFigmaClipboardRoute,
  importFigmaNodeRoute,
} from '@moatless/api-contract/figma'
import { clipboardPreset, generateFigmaPrompt, originalRestApiPreset } from '@moatless/figma'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/logger.js'

export function registerFigmaRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/figma/import
   * Import a Figma node by URL
   */
  app.openapi(importFigmaNodeRoute, async (c) => {
    try {
      const services = c.get('services')
      const { figma: figmaService } = services
      const { url } = c.req.valid('json')

      // Import the node using the service
      const result = await figmaService.importNode(url)

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
      const services = c.get('services')
      const { figma: figmaService } = services
      const { clipboardHtml } = c.req.valid('json')

      // Import clipboard using the service
      const result = await figmaService.importClipboard(clipboardHtml)

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
      const { storage } = c.get('services')

      // Get Figma import metadata
      const metadata = await storage.getFigmaImport(importId)

      if (!metadata) {
        throw new HTTPException(404, { message: 'Import not found' })
      }

      // Generate formatted prompt text for AI context
      const config = metadata.url ? originalRestApiPreset : clipboardPreset
      const prompt = generateFigmaPrompt(
        metadata.nodeMetadata,
        metadata.fileName,
        metadata.nodeName,
        config,
      )

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
      const services = c.get('services')
      const { storage, upload } = services

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
      const { buffer, metadata: uploadMetadata } = await upload.getFile(metadata.fileId)

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
}
