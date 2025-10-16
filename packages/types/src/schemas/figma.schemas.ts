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
    hasImage: z.boolean().describe('Whether this import has an associated image file'),
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
    url: z
      .string()
      .url()
      .optional()
      .describe('Original Figma URL (optional for clipboard imports)'),
    fileName: z.string().describe('Name of the Figma file'),
    nodeName: z.string().describe('Name of the imported node'),
    nodeId: z.string().describe('Node ID in Figma API format'),
    nodeMetadata: z.record(z.string(), z.unknown()).describe('Raw Figma node structure from API'),
    createdAt: z.string().describe('ISO 8601 timestamp of import'),
    hasImage: z.boolean().describe('Whether this import has an associated image file'),
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

/**
 * Request to import Figma clipboard data
 */
export const FigmaClipboardImportRequestSchema = z
  .object({
    clipboardHtml: z.string().describe('Raw HTML clipboard content from Figma'),
  })
  .describe('Request to import Figma clipboard data')

/**
 * Metadata extracted from Figma clipboard
 */
export const FigmaClipboardMetadataSchema = z
  .object({
    fileKey: z.string().describe('Figma file key'),
    pasteID: z.number().describe('Paste ID from Figma'),
    dataType: z.string().describe('Data type (e.g., "scene")'),
  })
  .describe('Metadata extracted from Figma clipboard')

/**
 * Parsed Figma node in tree structure
 * Uses recursive type with z.lazy for self-referencing nodes
 */
export type FigmaNode = {
  guid: string
  type: string
  name: string
  children?: FigmaNode[] | undefined
  properties?: Record<string, unknown> | undefined
  blobs?: Record<string, unknown> | undefined
}

export const FigmaNodeSchema: z.ZodType<FigmaNode> = z.lazy(() =>
  z.object({
    guid: z.string().describe('Node GUID'),
    type: z.string().describe('Node type (e.g., FRAME, TEXT, RECTANGLE)'),
    name: z.string().describe('Node name'),
    children: z.array(FigmaNodeSchema).optional().describe('Child nodes'),
    properties: z.record(z.string(), z.unknown()).optional().describe('Node properties'),
    blobs: z.record(z.string(), z.unknown()).optional().describe('Binary blob data'),
  }),
)

/**
 * Result of parsing Figma clipboard data
 */
export const FigmaClipboardParseResultSchema = z
  .object({
    metadata: FigmaClipboardMetadataSchema.describe('Clipboard metadata'),
    version: z.number().describe('Figma format version'),
    root: FigmaNodeSchema.describe('Root node of the design tree'),
    nodeCount: z.number().describe('Total number of nodes'),
    formatType: z
      .enum(['fig-kiwie', 'fig-jam.', 'fig-deck'])
      .describe('Format type (Figma, FigJam, or Slides)'),
  })
  .describe('Result of parsing Figma clipboard data')

// ===== Type Exports =====

export type FigmaImportRequest = z.infer<typeof FigmaImportRequestSchema>
export type FigmaImportResponse = z.infer<typeof FigmaImportResponseSchema>
export type FigmaImportMetadata = z.infer<typeof FigmaImportMetadataSchema>
export type FigmaImportIdParams = z.infer<typeof FigmaImportIdParamsSchema>
export type FigmaClipboardImportRequest = z.infer<typeof FigmaClipboardImportRequestSchema>
export type FigmaClipboardMetadata = z.infer<typeof FigmaClipboardMetadataSchema>
// FigmaNode type is defined above with the schema due to recursive nature
export type FigmaClipboardParseResult = z.infer<typeof FigmaClipboardParseResultSchema>

// ===== Figma Prompt Transformer Configuration =====

/**
 * Configuration for which sections to include in generated prompts
 */
export const FigmaPromptSectionsSchema = z
  .object({
    layout: z.boolean().optional().describe('Auto-layout/flexbox properties'),
    sizing: z.boolean().optional().describe('Sizing modes (hug/fixed)'),
    constraints: z.boolean().optional().describe('Positioning constraints'),
    typography: z.boolean().optional().describe('Text styling properties'),
    designTokens: z.boolean().optional().describe('Design token/variable indicators'),
    componentProps: z.boolean().optional().describe('Component variants and properties'),
    fills: z.boolean().optional().describe('Background colors'),
    strokes: z.boolean().optional().describe('Borders and strokes'),
    corners: z.boolean().optional().describe('Border radius'),
    effects: z.boolean().optional().describe('Shadows and blurs'),
    opacity: z.boolean().optional().describe('Transparency values'),
  })
  .describe('Configuration for which sections to include in generated prompts')

/**
 * Configuration for Figma prompt generation
 */
export const FigmaPromptConfigSchema = z
  .object({
    sections: FigmaPromptSectionsSchema.describe('Which sections to include'),
    propertyStyle: z
      .enum(['tailwind', 'figma'])
      .describe('Property naming style: Tailwind (gap-16px) or Figma (16px gap)'),
    maxTextLength: z.number().optional().describe('Maximum text content length before truncation'),
    showEmptyChildren: z.boolean().optional().describe('Whether to show nodes with no children'),
  })
  .describe('Configuration for Figma prompt generation')

/**
 * Preset configuration names
 */
export const FigmaPromptPresetSchema = z
  .enum(['originalRestApi', 'clipboard', 'minimal', 'detailed'])
  .describe('Preset configuration names')

// ===== Type Exports for Configuration =====

export type FigmaPromptSections = z.infer<typeof FigmaPromptSectionsSchema>
export type FigmaPromptConfig = z.infer<typeof FigmaPromptConfigSchema>
export type FigmaPromptPreset = z.infer<typeof FigmaPromptPresetSchema>
