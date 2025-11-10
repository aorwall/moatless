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
    url: z.url().describe('Figma file URL with node-id parameter'),
    includeImageInPrompt: z
      .boolean()
      .optional()
      .default(false)
      .describe('Whether to include the image in agent prompts (default: false)'),
  })
  .describe('Request to import a Figma node')

/**
 * Response containing import details
 */
export const FigmaImportResponseSchema = z
  .object({
    importId: z.uuid().describe('Unique identifier for this import'),
    fileId: z.uuid().describe('File ID for the uploaded image (same as importId)'),
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
    importId: z.uuid().describe('Unique identifier for this import'),
    fileId: z.uuid().describe('File ID for the uploaded image'),
    url: z.url().optional().describe('Original Figma URL (optional for clipboard imports)'),
    fileName: z.string().describe('Name of the Figma file'),
    nodeName: z.string().describe('Name of the imported node'),
    nodeId: z.string().describe('Node ID in Figma API format'),
    nodeMetadata: z.record(z.string(), z.unknown()).describe('Raw Figma node structure from API'),
    createdAt: z.string().describe('ISO 8601 timestamp of import'),
    hasImage: z.boolean().describe('Whether this import has an associated image file'),
    includeImageInPrompt: z.boolean().describe('Whether to include the image in agent prompts'),
    prompt: z.string().optional().describe('Formatted prompt text for AI context'),
  })
  .describe('Stored metadata for a Figma import')

/**
 * Route parameters for Figma import endpoints
 */
export const FigmaImportIdParamsSchema = z
  .object({
    importId: z.uuid().describe('Import identifier'),
  })
  .describe('Route parameters for Figma import endpoints')

/**
 * Request to import Figma clipboard data
 */
export const FigmaClipboardImportRequestSchema = z
  .object({
    clipboardHtml: z.string().describe('Raw HTML clipboard content from Figma'),
    includeImageInPrompt: z
      .boolean()
      .optional()
      .default(false)
      .describe(
        'Whether to include the image in agent prompts (always false for clipboard imports)',
      ),
  })
  .describe('Request to import Figma clipboard data')

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

// ===== Type Exports =====

export type FigmaImportMetadata = z.infer<typeof FigmaImportMetadataSchema>

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
    showUnusedComponents: z
      .boolean()
      .optional()
      .describe(
        'Whether to show component definitions that are not instantiated in the design (default: false)',
      ),
  })
  .describe('Configuration for Figma prompt generation')

// ===== Type Exports for Configuration =====

export type FigmaPromptConfig = z.infer<typeof FigmaPromptConfigSchema>

// ===== Figma OAuth Schemas =====

/**
 * Figma OAuth token response from Figma API
 */
export const FigmaOAuthTokenResponseSchema = z
  .object({
    access_token: z.string().describe('Access token for Figma API'),
    refresh_token: z.string().describe('Refresh token for renewing access'),
    expires_in: z.number().describe('Token expiration time in seconds'),
    user_id_string: z.string().describe('Figma user ID'),
  })
  .describe('Figma OAuth token response from Figma API')

/**
 * User-specific Figma configuration stored in database
 */
export const FigmaUserConfigSchema = z
  .object({
    userId: z.string().describe('User ID who owns this token'),
    authMethod: z
      .enum(['oauth', 'pat'])
      .describe('Authentication method: OAuth or Personal Access Token'),
    accessToken: z.string().describe('Figma access token'),
    refreshToken: z.string().optional().describe('Figma refresh token (OAuth only)'),
    expiresAt: z.string().optional().describe('ISO 8601 timestamp when token expires (OAuth only)'),
    connectedAt: z.string().describe('ISO 8601 timestamp when user connected'),
    figmaUserId: z.string().optional().describe('Figma user ID (OAuth only)'),
  })
  .describe('User-specific Figma configuration')

/**
 * Response containing Figma OAuth URL
 */
export const FigmaOAuthUrlResponseSchema = z
  .object({
    authUrl: z.url().describe('Figma OAuth authorization URL'),
    state: z.string().describe('CSRF protection state parameter'),
  })
  .describe('Response containing Figma OAuth URL')

/**
 * Request to exchange OAuth code for token
 */
export const FigmaOAuthTokenRequestSchema = z
  .object({
    code: z.string().describe('OAuth authorization code from Figma'),
    state: z.string().describe('CSRF protection state parameter'),
  })
  .describe('Request to exchange OAuth code for token')

/**
 * PKCE verifier stored temporarily during OAuth flow
 */
export const FigmaPKCEVerifierSchema = z
  .object({
    verifier: z.string().describe('PKCE code verifier'),
    state: z.string().describe('OAuth state parameter'),
    userId: z.string().describe('User ID initiating the OAuth flow'),
    createdAt: z.string().describe('ISO 8601 timestamp when verifier was created'),
  })
  .describe('PKCE verifier stored temporarily during OAuth flow')

/**
 * Request to save Personal Access Token
 */
export const FigmaPATRequestSchema = z
  .object({
    accessToken: z.string().min(1).describe('Figma Personal Access Token'),
  })
  .describe('Request to save Personal Access Token')

// ===== Type Exports for OAuth =====

export type FigmaOAuthTokenResponse = z.infer<typeof FigmaOAuthTokenResponseSchema>
export type FigmaUserConfig = z.infer<typeof FigmaUserConfigSchema>
export type FigmaOAuthUrlResponse = z.infer<typeof FigmaOAuthUrlResponseSchema>
export type FigmaOAuthTokenRequest = z.infer<typeof FigmaOAuthTokenRequestSchema>
export type FigmaPKCEVerifier = z.infer<typeof FigmaPKCEVerifierSchema>
export type FigmaPATRequest = z.infer<typeof FigmaPATRequestSchema>
