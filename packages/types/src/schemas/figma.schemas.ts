import { z } from 'zod'

// Re-export official Figma API types
export type {
  BlendMode,
  BlurEffect,
  ColorStop,
  ComponentNode,
  Effect,
  EllipseNode,
  FrameNode,
  GradientPaint,
  GroupNode,
  ImagePaint,
  InstanceNode,
  LineNode,
  Node,
  NoiseEffect,
  Paint,
  PatternPaint,
  RectangleNode,
  RegularPolygonNode,
  RGBA,
  SolidPaint,
  StarNode,
  TextNode,
  Transform,
  VectorNode,
} from '@figma/rest-api-spec'

// ===== Application-Specific Schemas (Not from Figma API) =====

/**
 * Request to import a Figma node
 */
export const FigmaImportRequestSchema = z
  .object({
    url: z.string().url().describe('Figma file URL with node-id parameter'),
  })
  .describe('Request to import a Figma node')

/**
 * Response containing import details
 */
export const FigmaImportResponseSchema = z
  .object({
    importId: z.string().uuid().describe('Unique identifier for this import'),
    fileId: z.string().uuid().describe('File ID for the uploaded image (same as importId)'),
    nodeId: z.string().describe('Node ID in Figma API format (e.g., 3376:185062)'),
    fileName: z.string().describe('Name of the Figma file'),
    nodeName: z.string().describe('Name of the imported node'),
    createdAt: z.string().describe('ISO 8601 timestamp of import'),
  })
  .describe('Response containing import details')

/**
 * Stored metadata for a Figma import
 * This schema uses a passthrough record for node metadata to accept any node type from Figma
 */
export const FigmaImportMetadataSchema = z
  .object({
    importId: z.string().uuid().describe('Unique identifier for this import'),
    fileId: z.string().uuid().describe('File ID for the uploaded image'),
    url: z.string().url().describe('Original Figma URL'),
    fileName: z.string().describe('Name of the Figma file'),
    nodeName: z.string().describe('Name of the imported node'),
    nodeId: z.string().describe('Node ID in Figma API format'),
    nodeMetadata: z.record(z.string(), z.unknown()).describe('Raw Figma node structure from API'),
    createdAt: z.string().describe('ISO 8601 timestamp of import'),
    prompt: z.string().optional().describe('Formatted prompt text for AI context'),
  })
  .describe('Stored metadata for a Figma import')

/**
 * Route parameters for Figma import endpoints
 */
export const FigmaImportIdParamsSchema = z
  .object({
    importId: z.string().uuid().describe('Import identifier'),
  })
  .describe('Route parameters for Figma import endpoints')

// ===== Type Exports =====

export type FigmaImportRequest = z.infer<typeof FigmaImportRequestSchema>
export type FigmaImportResponse = z.infer<typeof FigmaImportResponseSchema>
export type FigmaImportMetadata = z.infer<typeof FigmaImportMetadataSchema>
export type FigmaImportIdParams = z.infer<typeof FigmaImportIdParamsSchema>
