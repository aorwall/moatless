import { randomUUID } from 'node:crypto'
import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  getFigmaImportImageRoute,
  getFigmaImportRoute,
  importFigmaNodeRoute,
} from '@moatless/api-contract/figma'
import type { FigmaImportMetadata } from '@moatless/types'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { FigmaService } from '../services/figma.service.js'
import { logger } from '../utils/logger.js'

export function registerFigmaRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/figma/import
   * Import a Figma node by URL
   */
  app.openapi(importFigmaNodeRoute, async (c) => {
    try {
      const services = c.get('services')
      const { storage, upload, figma: figmaService } = services
      const { url } = c.req.valid('json')

      // Parse Figma URL to extract fileKey and nodeId
      const fileKey = FigmaService.extractFileKey(url)
      const nodeId = FigmaService.extractNodeId(url)

      if (!(fileKey && nodeId)) {
        throw new HTTPException(400, {
          message: 'Invalid Figma URL. Must include file key and node-id parameter.',
        })
      }

      // Fetch node metadata
      const nodeData = await figmaService.getNode(fileKey, nodeId)
      if (!nodeData) {
        throw new HTTPException(500, {
          message: 'Failed to fetch node data from Figma API',
        })
      }

      // Fetch node image URL
      const imageUrl = await figmaService.getNodeImage(fileKey, nodeId, {
        format: 'png',
        scale: 1,
      })

      if (!imageUrl) {
        throw new HTTPException(500, {
          message: 'Failed to fetch image URL from Figma API',
        })
      }

      // Download image from Figma CDN
      const imageResponse = await fetch(imageUrl)
      if (!imageResponse.ok) {
        throw new HTTPException(500, {
          message: `Failed to download image from Figma CDN: ${imageResponse.statusText}`,
        })
      }

      const imageBuffer = Buffer.from(await imageResponse.arrayBuffer())

      // Generate UUID for import (same for both importId and fileId)
      const importId = randomUUID()

      // Save image using upload service
      const filename = `${nodeData.simplified.name.replace(/[^a-z0-9]/gi, '-')}.png`
      await upload.uploadFile(imageBuffer, filename, 'image/png')

      // Note: We're using importId as fileId because the upload service uses
      // the random UUID generated internally. We need to directly write the file
      // with our UUID instead.
      // Let me fix this by using the upload service's internal methods
      const uploadDir = process.env['UPLOAD_DIR'] || upload['uploadDir']
      const metadataDir = upload['metadataDir']

      const promises = await import('node:fs/promises')
      const path = await import('node:path')

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

      // Create Figma import metadata
      const figmaMetadata: FigmaImportMetadata = {
        importId,
        fileId: importId,
        url,
        fileName: nodeData.fileName,
        nodeName: nodeData.simplified.name,
        nodeId,
        nodeMetadata: nodeData.simplified,
        createdAt: new Date().toISOString(),
      }

      // Save Figma metadata
      await storage.saveFigmaImport(figmaMetadata)

      logger.info('Figma node imported successfully', {
        importId,
        fileId: importId,
        nodeId,
        fileName: nodeData.fileName,
        nodeName: nodeData.simplified.name,
      })

      // Return response
      return c.json({
        importId,
        fileId: importId,
        nodeId,
        fileName: nodeData.fileName,
        nodeName: nodeData.simplified.name,
        createdAt: figmaMetadata.createdAt,
      })
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

      return c.json(metadata)
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
