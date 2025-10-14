import { z } from 'zod'

// ===== Core Figma Type Enums =====

export const FigmaNodeTypeSchema = z
  .enum([
    'DOCUMENT',
    'CANVAS',
    'FRAME',
    'GROUP',
    'VECTOR',
    'BOOLEAN_OPERATION',
    'STAR',
    'LINE',
    'ELLIPSE',
    'REGULAR_POLYGON',
    'RECTANGLE',
    'TEXT',
    'SLICE',
    'COMPONENT',
    'COMPONENT_SET',
    'INSTANCE',
    'STICKY',
    'SHAPE_WITH_TEXT',
    'CONNECTOR',
  ])
  .describe('Node type in Figma. Determines the structure and available properties of the node.')

export const FigmaLayoutModeSchema = z
  .enum(['NONE', 'HORIZONTAL', 'VERTICAL'])
  .describe(
    'Determines auto-layout direction. NONE: no auto-layout, HORIZONTAL: horizontal stack, VERTICAL: vertical stack',
  )

export const FigmaPrimaryAxisSizingModeSchema = z
  .enum(['FIXED', 'AUTO'])
  .describe(
    "How the frame's primary axis (main direction) sizes itself. FIXED: fixed size, AUTO: hugs contents",
  )

export const FigmaCounterAxisSizingModeSchema = z
  .enum(['FIXED', 'AUTO'])
  .describe(
    "How the frame's counter axis (cross direction) sizes itself. FIXED: fixed size, AUTO: hugs contents",
  )

export const FigmaLayoutAlignSchema = z
  .enum(['MIN', 'CENTER', 'MAX', 'STRETCH', 'INHERIT'])
  .describe(
    'Alignment in auto-layout. MIN: start, CENTER: center, MAX: end, STRETCH: fill space, INHERIT: inherit from parent',
  )

export const FigmaBlendModeSchema = z
  .enum([
    'PASS_THROUGH',
    'NORMAL',
    'DARKEN',
    'MULTIPLY',
    'LINEAR_BURN',
    'COLOR_BURN',
    'LIGHTEN',
    'SCREEN',
    'LINEAR_DODGE',
    'COLOR_DODGE',
    'OVERLAY',
    'SOFT_LIGHT',
    'HARD_LIGHT',
    'DIFFERENCE',
    'EXCLUSION',
    'HUE',
    'SATURATION',
    'COLOR',
    'LUMINOSITY',
  ])
  .describe('How this node blends with nodes behind it')

// ===== Primitive Schemas =====

export const FigmaColorSchema = z
  .object({
    r: z.number().min(0).max(1).describe('Red channel (0-1)'),
    g: z.number().min(0).max(1).describe('Green channel (0-1)'),
    b: z.number().min(0).max(1).describe('Blue channel (0-1)'),
    a: z.number().min(0).max(1).optional().describe('Alpha/opacity channel (0-1), defaults to 1'),
  })
  .describe('RGBA color with values normalized to 0-1 range')

export const FigmaRectangleSchema = z
  .object({
    x: z.number().describe('X coordinate'),
    y: z.number().describe('Y coordinate'),
    width: z.number().describe('Width in pixels'),
    height: z.number().describe('Height in pixels'),
  })
  .describe('Bounding box rectangle with position and dimensions')

export const FigmaVector2Schema = z
  .object({
    x: z.number().describe('X coordinate or width'),
    y: z.number().describe('Y coordinate or height'),
  })
  .describe('2D vector for positions, sizes, and offsets')

// ===== Paint Schemas =====

export const FigmaPaintTypeSchema = z
  .enum([
    'SOLID',
    'GRADIENT_LINEAR',
    'GRADIENT_RADIAL',
    'GRADIENT_ANGULAR',
    'GRADIENT_DIAMOND',
    'IMAGE',
    'EMOJI',
  ])
  .describe('Type of paint (fill or stroke)')

export const FigmaPaintSchema = z
  .object({
    type: FigmaPaintTypeSchema.describe('Paint type'),
    visible: z.boolean().optional().describe('Whether paint is visible, defaults to true'),
    opacity: z.number().min(0).max(1).optional().describe('Opacity of paint (0-1), defaults to 1'),
    color: FigmaColorSchema.optional().describe('Color for solid paints'),
    blendMode: FigmaBlendModeSchema.optional().describe('How paint blends with layers behind it'),
  })
  .describe(
    'Paint applied to fills or strokes. Simplified schema - full API includes gradients, images, etc.',
  )

// ===== Effect Schemas =====

export const FigmaEffectTypeSchema = z
  .enum(['INNER_SHADOW', 'DROP_SHADOW', 'LAYER_BLUR', 'BACKGROUND_BLUR'])
  .describe('Type of visual effect')

export const FigmaEffectSchema = z
  .object({
    type: FigmaEffectTypeSchema.describe('Effect type'),
    visible: z.boolean().optional().describe('Whether effect is visible, defaults to true'),
    radius: z.number().optional().describe('Blur radius for shadows and blurs'),
    color: FigmaColorSchema.optional().describe('Color for shadow effects'),
    offset: FigmaVector2Schema.optional().describe('Offset for shadow effects'),
    spread: z.number().optional().describe('Spread for shadow effects'),
    blendMode: FigmaBlendModeSchema.optional().describe('How effect blends'),
  })
  .describe('Visual effect like shadows and blurs')

// ===== Layout Schemas =====

export const FigmaLayoutPropertiesSchema = z
  .object({
    layoutMode: FigmaLayoutModeSchema.optional().describe('Auto-layout direction'),
    primaryAxisSizingMode: FigmaPrimaryAxisSizingModeSchema.optional().describe(
      'How primary axis sizes (FIXED or AUTO)',
    ),
    counterAxisSizingMode: FigmaCounterAxisSizingModeSchema.optional().describe(
      'How counter axis sizes (FIXED or AUTO)',
    ),
    primaryAxisAlignItems: FigmaLayoutAlignSchema.optional().describe(
      'Alignment of children along primary axis',
    ),
    counterAxisAlignItems: FigmaLayoutAlignSchema.optional().describe(
      'Alignment of children along counter axis',
    ),
    paddingLeft: z.number().optional().describe('Left padding in pixels'),
    paddingRight: z.number().optional().describe('Right padding in pixels'),
    paddingTop: z.number().optional().describe('Top padding in pixels'),
    paddingBottom: z.number().optional().describe('Bottom padding in pixels'),
    itemSpacing: z.number().optional().describe('Spacing between items in auto-layout'),
    counterAxisSpacing: z.number().optional().describe('Spacing in counter axis direction'),
    layoutAlign: FigmaLayoutAlignSchema.optional().describe(
      'How this node aligns within parent auto-layout',
    ),
    layoutGrow: z
      .number()
      .optional()
      .describe('Whether this node should stretch to fill parent (0 or 1)'),
  })
  .describe('Auto-layout properties for frames')

// ===== Constraints Schema =====

export const FigmaConstraintTypeSchema = z
  .enum(['MIN', 'CENTER', 'MAX', 'STRETCH', 'SCALE'])
  .describe('Constraint type for positioning')

export const FigmaConstraintsSchema = z
  .object({
    vertical: FigmaConstraintTypeSchema.optional().describe('Vertical constraint'),
    horizontal: FigmaConstraintTypeSchema.optional().describe('Horizontal constraint'),
  })
  .describe('Layout constraints for positioning when parent resizes')

// ===== Typography Schema =====

export const FigmaTextAlignHorizontalSchema = z
  .enum(['LEFT', 'RIGHT', 'CENTER', 'JUSTIFIED'])
  .describe('Horizontal text alignment')

export const FigmaTextAlignVerticalSchema = z
  .enum(['TOP', 'CENTER', 'BOTTOM'])
  .describe('Vertical text alignment')

export const FigmaTypographySchema = z
  .object({
    fontFamily: z.string().optional().describe('Font family name'),
    fontPostScriptName: z.string().optional().describe('PostScript font name'),
    fontSize: z.number().optional().describe('Font size in pixels'),
    fontWeight: z.number().optional().describe('Font weight (100-900)'),
    textAlignHorizontal: FigmaTextAlignHorizontalSchema.optional().describe(
      'Horizontal text alignment',
    ),
    textAlignVertical: FigmaTextAlignVerticalSchema.optional().describe('Vertical text alignment'),
    letterSpacing: z.number().optional().describe('Letter spacing in pixels'),
    lineHeightPx: z.number().optional().describe('Line height in pixels'),
    lineHeightPercent: z.number().optional().describe('Line height as percentage of font size'),
    characters: z.string().optional().describe('Text content'),
  })
  .describe('Typography properties for TEXT nodes')

// ===== Simplified Node Schema =====

export const SimplifiedFigmaNodeSchema: z.ZodType<SimplifiedFigmaNode> = z
  .object({
    // Core properties - REQUIRED
    id: z.string().describe('Unique identifier for this node'),
    name: z.string().describe('Name of the node set in Figma'),
    type: FigmaNodeTypeSchema.describe('Node type'),
    visible: z.boolean().default(true).describe('Whether node is visible, defaults to true'),

    // Bounds and sizing - OPTIONAL
    absoluteBoundingBox: FigmaRectangleSchema.optional().describe(
      'Bounding box in absolute file coordinates',
    ),
    size: FigmaVector2Schema.optional().describe('Width and height of the node'),

    // Layout (auto-layout) - OPTIONAL
    layout: FigmaLayoutPropertiesSchema.optional().describe(
      'Auto-layout properties if node uses auto-layout',
    ),

    // Visual styling - OPTIONAL
    fills: z.array(FigmaPaintSchema).optional().describe('Array of fill paints, defaults to []'),
    strokes: z
      .array(FigmaPaintSchema)
      .optional()
      .describe('Array of stroke paints, defaults to []'),
    effects: z
      .array(FigmaEffectSchema)
      .optional()
      .describe('Array of effects like shadows and blurs'),
    opacity: z.number().min(0).max(1).optional().describe('Overall opacity (0-1), defaults to 1'),
    blendMode: FigmaBlendModeSchema.optional().describe('How node blends with layers behind it'),
    isMask: z.boolean().optional().describe('Whether node is used as a mask'),

    // Typography (for TEXT nodes) - OPTIONAL
    typography: FigmaTypographySchema.optional().describe('Typography properties for TEXT nodes'),

    // Constraints - OPTIONAL
    constraints: FigmaConstraintsSchema.optional().describe('Layout constraints for resizing'),

    // Hierarchy - OPTIONAL
    children: z
      .array(z.lazy(() => SimplifiedFigmaNodeSchema))
      .optional()
      .describe('Child nodes'),

    // Metadata - OPTIONAL
    exportSettings: z.array(z.unknown()).optional().describe('Export settings for this node'),
  })
  .describe('Simplified Figma node structure with only essential properties for AI coding contexts')

// Type must be defined before schema due to recursion
export type SimplifiedFigmaNode = {
  id: string
  name: string
  type: z.infer<typeof FigmaNodeTypeSchema>
  visible: boolean
  absoluteBoundingBox?: FigmaRectangle | undefined
  size?: z.infer<typeof FigmaVector2Schema> | undefined
  layout?: FigmaLayoutProperties | undefined
  fills?: FigmaPaint[] | undefined
  strokes?: FigmaPaint[] | undefined
  effects?: FigmaEffect[] | undefined
  opacity?: number | undefined
  blendMode?: z.infer<typeof FigmaBlendModeSchema> | undefined
  isMask?: boolean | undefined
  typography?: FigmaTypography | undefined
  constraints?: FigmaConstraints | undefined
  children?: SimplifiedFigmaNode[] | undefined
  exportSettings?: unknown[] | undefined
}

// ===== Raw Figma Node (from API) =====

export const RawFigmaNodeSchema: z.ZodType<RawFigmaNode> = z
  .object({
    id: z.string().describe('Unique node identifier'),
    name: z.string().describe('Node name'),
    type: z.string().describe('Node type string'),
    visible: z.boolean().optional().describe('Whether node is visible'),
    children: z
      .array(z.lazy(() => RawFigmaNodeSchema))
      .optional()
      .describe('Child nodes'),
  })
  .passthrough()
  .describe('Raw Figma node from API with all original properties preserved')

// Type must be defined before schema due to recursion
export type RawFigmaNode = {
  id: string
  name: string
  type: string
  visible?: boolean | undefined
  children?: RawFigmaNode[] | undefined
  [key: string]: unknown
}

// ===== Figma Node Response Schema =====

export const FigmaNodeResponseSchema = z
  .object({
    name: z.string().describe('File name'),
    role: z.string().describe('User role in file (e.g., owner, editor, viewer)'),
    lastModified: z.string().describe('ISO 8601 timestamp of last modification'),
    nodes: z
      .record(
        z.string(),
        z
          .object({
            document: RawFigmaNodeSchema.describe('Node document tree'),
          })
          .nullable(),
      )
      .describe('Map of node IDs to node data, null if node not found'),
  })
  .describe('Response from GET /v1/files/:key/nodes endpoint')

// ===== Parsed Node Result =====

export const ParsedFigmaNodeSchema = z
  .object({
    simplified: SimplifiedFigmaNodeSchema.describe('Simplified node with essential properties'),
    raw: RawFigmaNodeSchema.describe('Raw node from API with all properties'),
    fileName: z.string().describe('Name of the Figma file'),
  })
  .describe('Parsed node result containing both simplified and raw formats')

// ===== Figma Import Schemas =====

export const FigmaImportRequestSchema = z
  .object({
    url: z.string().url().describe('Figma file URL with node-id parameter'),
  })
  .describe('Request to import a Figma node')

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

export const FigmaImportMetadataSchema = z
  .object({
    importId: z.string().uuid().describe('Unique identifier for this import'),
    fileId: z.string().uuid().describe('File ID for the uploaded image'),
    url: z.string().url().describe('Original Figma URL'),
    fileName: z.string().describe('Name of the Figma file'),
    nodeName: z.string().describe('Name of the imported node'),
    nodeId: z.string().describe('Node ID in Figma API format'),
    nodeMetadata: SimplifiedFigmaNodeSchema.describe('Simplified Figma node structure'),
    createdAt: z.string().describe('ISO 8601 timestamp of import'),
  })
  .describe('Stored metadata for a Figma import')

export const FigmaImportIdParamsSchema = z
  .object({
    importId: z.string().uuid().describe('Import identifier'),
  })
  .describe('Route parameters for Figma import endpoints')

// ===== Type Exports =====

export type FigmaNodeType = z.infer<typeof FigmaNodeTypeSchema>
export type FigmaLayoutMode = z.infer<typeof FigmaLayoutModeSchema>
export type FigmaBlendMode = z.infer<typeof FigmaBlendModeSchema>
export type FigmaColor = z.infer<typeof FigmaColorSchema>
export type FigmaRectangle = z.infer<typeof FigmaRectangleSchema>
export type FigmaPaint = z.infer<typeof FigmaPaintSchema>
export type FigmaEffect = z.infer<typeof FigmaEffectSchema>
export type FigmaLayoutProperties = z.infer<typeof FigmaLayoutPropertiesSchema>
export type FigmaConstraints = z.infer<typeof FigmaConstraintsSchema>
export type FigmaTypography = z.infer<typeof FigmaTypographySchema>
// SimplifiedFigmaNode and RawFigmaNode are defined above the schemas
export type FigmaNodeResponse = z.infer<typeof FigmaNodeResponseSchema>
export type ParsedFigmaNode = z.infer<typeof ParsedFigmaNodeSchema>
export type FigmaImportRequest = z.infer<typeof FigmaImportRequestSchema>
export type FigmaImportResponse = z.infer<typeof FigmaImportResponseSchema>
export type FigmaImportMetadata = z.infer<typeof FigmaImportMetadataSchema>
export type FigmaImportIdParams = z.infer<typeof FigmaImportIdParamsSchema>
