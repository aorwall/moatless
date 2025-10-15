import type {
  FigmaEffect,
  FigmaLayoutProperties,
  FigmaPaint,
  FigmaTypography,
  SimplifiedFigmaNode,
} from '@moatless/types'

/**
 * Format layout information into readable text
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Layout formatting requires checking multiple conditional properties
export function formatLayoutInfo(layout: FigmaLayoutProperties): string {
  const parts: string[] = []

  // Layout mode
  if (layout.layoutMode === 'HORIZONTAL') {
    parts.push('Horizontal auto-layout')
  } else if (layout.layoutMode === 'VERTICAL') {
    parts.push('Vertical auto-layout')
  }

  // Layout positioning
  if (layout.layoutPositioning === 'ABSOLUTE') {
    parts.push('absolute positioning')
  } else if (layout.layoutPositioning === 'AUTO') {
    parts.push('auto positioning')
  }

  // Layout sizing modes
  const sizingParts: string[] = []
  if (layout.layoutSizingHorizontal) {
    const sizingMap: Record<string, string> = {
      FIXED: 'fixed width',
      HUG: 'hug width',
      FILL: 'fill width',
    }
    const horizontalText = sizingMap[layout.layoutSizingHorizontal]
    if (horizontalText) {
      sizingParts.push(horizontalText)
    }
  }
  if (layout.layoutSizingVertical) {
    const sizingMap: Record<string, string> = {
      FIXED: 'fixed height',
      HUG: 'hug height',
      FILL: 'fill height',
    }
    const verticalText = sizingMap[layout.layoutSizingVertical]
    if (verticalText) {
      sizingParts.push(verticalText)
    }
  }
  if (sizingParts.length > 0) {
    parts.push(sizingParts.join(', '))
  }

  // Alignment
  if (layout.primaryAxisAlignItems && layout.primaryAxisAlignItems !== 'MIN') {
    const alignMap: Record<string, string> = {
      CENTER: 'center aligned',
      MAX: 'end aligned',
      SPACE_BETWEEN: 'space-between',
      STRETCH: 'stretch',
      INHERIT: 'inherit alignment',
    }
    parts.push(alignMap[layout.primaryAxisAlignItems] || layout.primaryAxisAlignItems)
  }

  // Spacing
  if (layout.itemSpacing) {
    parts.push(`${layout.itemSpacing}px gap`)
  }

  // Padding
  if (layout.paddingTop || layout.paddingRight || layout.paddingBottom || layout.paddingLeft) {
    const padding = [
      layout.paddingTop || 0,
      layout.paddingRight || 0,
      layout.paddingBottom || 0,
      layout.paddingLeft || 0,
    ]
    if (padding.every((p) => p === padding[0])) {
      parts.push(`${padding[0]}px padding`)
    } else {
      parts.push(`padding: ${padding.join('px ')}px`)
    }
  }

  return parts.length > 0 ? parts.join(', ') : 'No auto-layout'
}

/**
 * Format typography information into readable text
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Typography formatting requires checking multiple style properties
export function formatTypographyInfo(typography: FigmaTypography, characters?: string): string {
  const parts: string[] = []

  // Text content (truncated if too long)
  if (characters) {
    const truncated = characters.length > 100 ? `${characters.substring(0, 100)}...` : characters
    parts.push(`"${truncated}"`)
  }

  // Font info
  const fontParts: string[] = []
  if (typography.fontSize) {
    fontParts.push(`${typography.fontSize}px`)
  }
  if (typography.fontWeight) {
    const weightMap: Record<number, string> = {
      100: 'Thin',
      200: 'Extra Light',
      300: 'Light',
      400: 'Regular',
      500: 'Medium',
      600: 'Semi Bold',
      700: 'Bold',
      800: 'Extra Bold',
      900: 'Black',
    }
    fontParts.push(weightMap[typography.fontWeight] || `${typography.fontWeight}`)
  }
  if (typography.fontFamily) {
    fontParts.push(typography.fontFamily)
  }

  if (fontParts.length > 0) {
    parts.push(`(${fontParts.join(', ')})`)
  }

  // Text styling
  const styleParts: string[] = []
  if (typography.textCase && typography.textCase !== 'ORIGINAL') {
    const caseMap: Record<string, string> = {
      UPPER: 'uppercase',
      LOWER: 'lowercase',
      TITLE: 'title case',
      SMALL_CAPS: 'small caps',
      SMALL_CAPS_FORCED: 'small caps forced',
      ORIGINAL: '',
    }
    const caseText = caseMap[typography.textCase]
    if (caseText) {
      styleParts.push(caseText)
    }
  }
  if (typography.textDecoration && typography.textDecoration !== 'NONE') {
    const decorationMap: Record<string, string> = {
      STRIKETHROUGH: 'strikethrough',
      UNDERLINE: 'underline',
      NONE: '',
    }
    const decorationText = decorationMap[typography.textDecoration]
    if (decorationText) {
      styleParts.push(decorationText)
    }
  }
  if (styleParts.length > 0) {
    parts.push(`[${styleParts.join(', ')}]`)
  }

  // Text behavior
  if (typography.textAutoResize) {
    const autoResizeMap: Record<string, string> = {
      WIDTH_AND_HEIGHT: 'auto-resize both',
      HEIGHT: 'auto-resize height',
      TRUNCATE: 'truncate',
      NONE: '',
    }
    const autoResizeText = autoResizeMap[typography.textAutoResize]
    if (autoResizeText) {
      parts.push(`{${autoResizeText}}`)
    }
  }

  // Hyperlink
  if (typography.hyperlink) {
    parts.push(`(link: ${typography.hyperlink.url})`)
  }

  return parts.join(' ')
}

/**
 * Format color from Figma color object to hex string
 */
function formatColor(color: { r: number; g: number; b: number; a?: number | undefined }): string {
  const r = Math.round(color.r * 255)
  const g = Math.round(color.g * 255)
  const b = Math.round(color.b * 255)
  const hex =
    `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`.toUpperCase()

  if (color.a !== undefined && color.a < 1) {
    return `${hex} (${Math.round(color.a * 100)}% opacity)`
  }

  return hex
}

/**
 * Format paint (fill or stroke) information into readable text
 */
export function formatPaintInfo(paints: FigmaPaint[]): string {
  if (!paints || paints.length === 0) {
    return 'none'
  }

  const visiblePaints = paints.filter((p) => p.visible !== false)
  if (visiblePaints.length === 0) {
    return 'none'
  }

  return visiblePaints
    .map((paint) => {
      if (paint.type === 'SOLID' && paint.color) {
        return formatColor(paint.color)
      }
      if (paint.type === 'IMAGE') {
        return 'Image fill'
      }
      if (paint.type.startsWith('GRADIENT_')) {
        return `${paint.type.replace('GRADIENT_', '')} gradient`
      }
      return paint.type
    })
    .join(', ')
}

/**
 * Format effects (shadows, blurs) into readable text
 */
export function formatEffectsInfo(effects: FigmaEffect[]): string {
  if (!effects || effects.length === 0) {
    return ''
  }

  const visibleEffects = effects.filter((e) => e.visible !== false)
  if (visibleEffects.length === 0) {
    return ''
  }

  return visibleEffects
    .map((effect) => {
      const parts: string[] = []

      if (effect.type === 'DROP_SHADOW') {
        parts.push('Drop shadow')
        if (effect.radius) {
          parts.push(`${effect.radius}px blur`)
        }
        if (effect.color) {
          parts.push(formatColor(effect.color))
        }
      } else if (effect.type === 'INNER_SHADOW') {
        parts.push('Inner shadow')
        if (effect.radius) {
          parts.push(`${effect.radius}px blur`)
        }
      } else if (effect.type === 'LAYER_BLUR') {
        parts.push(`Layer blur ${effect.radius || 0}px`)
      } else if (effect.type === 'BACKGROUND_BLUR') {
        parts.push(`Background blur ${effect.radius || 0}px`)
      }

      return parts.join(' ')
    })
    .join(', ')
}

/**
 * Format stroke (border) properties into readable text
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Stroke formatting requires checking multiple border/line properties
export function formatStrokeProperties(
  strokeProps: {
    strokeWeight?: number | undefined
    strokeAlign?: 'INSIDE' | 'OUTSIDE' | 'CENTER' | undefined
    strokeCap?: 'NONE' | 'ROUND' | 'SQUARE' | 'LINE_ARROW' | 'TRIANGLE_ARROW' | undefined
    strokeJoin?: 'MITER' | 'BEVEL' | 'ROUND' | undefined
    strokeDashes?: number[] | undefined
    strokeMiterLimit?: number | undefined
    individualStrokeWeights?:
      | { top: number; right: number; bottom: number; left: number }
      | undefined
  },
  strokes?: FigmaPaint[],
): string {
  const parts: string[] = []

  // Individual stroke weights (different border widths per side)
  if (strokeProps.individualStrokeWeights) {
    const { top, right, bottom, left } = strokeProps.individualStrokeWeights
    if (top === right && right === bottom && bottom === left) {
      parts.push(`${top}px border`)
    } else {
      parts.push(`border (${top}px ${right}px ${bottom}px ${left}px)`)
    }
  } else if (strokeProps.strokeWeight) {
    parts.push(`${strokeProps.strokeWeight}px border`)
  }

  // Stroke alignment
  if (strokeProps.strokeAlign) {
    const alignMap: Record<string, string> = {
      INSIDE: 'inside',
      OUTSIDE: 'outside',
      CENTER: 'center',
    }
    parts.push(`${alignMap[strokeProps.strokeAlign]} aligned`)
  }

  // Stroke color
  if (strokes && strokes.length > 0) {
    const strokeColor = formatPaintInfo(strokes)
    if (strokeColor !== 'none') {
      parts.push(strokeColor)
    }
  }

  // Dashed borders
  if (strokeProps.strokeDashes && strokeProps.strokeDashes.length > 0) {
    parts.push(`dashed (${strokeProps.strokeDashes.join(' ')})`)
  }

  // Line cap style
  if (strokeProps.strokeCap && strokeProps.strokeCap !== 'NONE') {
    const capMap: Record<string, string> = {
      ROUND: 'round cap',
      SQUARE: 'square cap',
      LINE_ARROW: 'arrow cap',
      TRIANGLE_ARROW: 'triangle arrow cap',
      NONE: '',
    }
    const capText = capMap[strokeProps.strokeCap]
    if (capText) {
      parts.push(capText)
    }
  }

  // Corner join style
  if (strokeProps.strokeJoin) {
    const joinMap: Record<string, string> = {
      MITER: 'miter join',
      BEVEL: 'bevel join',
      ROUND: 'round join',
    }
    const joinText = joinMap[strokeProps.strokeJoin]
    if (joinText) {
      parts.push(joinText)
    }
  }

  return parts.length > 0 ? parts.join(', ') : ''
}

/**
 * Format corner radius properties into readable text
 */
export function formatCornerProperties(cornerProps: {
  cornerRadius?: number | undefined
  cornerSmoothing?: number | undefined
  rectangleCornerRadii?: [number, number, number, number] | undefined
}): string {
  const parts: string[] = []

  if (cornerProps.rectangleCornerRadii) {
    const [tl, tr, br, bl] = cornerProps.rectangleCornerRadii
    if (tl === tr && tr === br && br === bl) {
      parts.push(`${tl}px radius`)
    } else {
      parts.push(`radius (${tl}px ${tr}px ${br}px ${bl}px)`)
    }
  } else if (cornerProps.cornerRadius) {
    parts.push(`${cornerProps.cornerRadius}px radius`)
  }

  if (cornerProps.cornerSmoothing) {
    parts.push(`${Math.round(cornerProps.cornerSmoothing * 100)}% smoothing`)
  }

  return parts.length > 0 ? parts.join(', ') : ''
}

/**
 * Format sizing constraints into readable text
 */
export function formatSizingConstraints(constraints: {
  minWidth?: number | undefined
  maxWidth?: number | undefined
  minHeight?: number | undefined
  maxHeight?: number | undefined
}): string {
  const parts: string[] = []

  if (constraints.minWidth) {
    parts.push(`min-width ${constraints.minWidth}px`)
  }
  if (constraints.maxWidth) {
    parts.push(`max-width ${constraints.maxWidth}px`)
  }
  if (constraints.minHeight) {
    parts.push(`min-height ${constraints.minHeight}px`)
  }
  if (constraints.maxHeight) {
    parts.push(`max-height ${constraints.maxHeight}px`)
  }

  return parts.length > 0 ? parts.join(', ') : ''
}

/**
 * Format dimensions into readable text
 */
export function formatDimensions(node: SimplifiedFigmaNode): string {
  if (node.absoluteBoundingBox) {
    const { width, height } = node.absoluteBoundingBox
    return `${Math.round(width)}x${Math.round(height)}px`
  }
  if (node.size > 0) {
    return `${Math.round(node.size.x)}x${Math.round(node.size.y)}px`
  }
  return ''
}

/**
 * Format a single node into readable text with indentation
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Node formatting requires checking all possible node properties and styling
function formatNode(node: SimplifiedFigmaNode, indent = 0): string[] {
  const lines: string[] = []
  const prefix = '  '.repeat(indent)
  const arrow = indent > 0 ? '→ ' : ''

  // Node header with type, name, and dimensions
  const dimensions = formatDimensions(node)
  const header = `${prefix}${arrow}${node.type} "${node.name}"${dimensions ? ` (${dimensions})` : ''}`
  lines.push(header)

  // Layout info
  if (node.layout) {
    const layoutInfo = formatLayoutInfo(node.layout)
    if (layoutInfo && layoutInfo !== 'No auto-layout') {
      lines.push(`${prefix}  - Layout: ${layoutInfo}`)
    }
  }

  // Typography info (for TEXT nodes)
  if (node.type === 'TEXT' && node.typography) {
    const typographyInfo = formatTypographyInfo(node.typography, node.typography.characters)
    if (typographyInfo) {
      lines.push(`${prefix}  - Text: ${typographyInfo}`)
    }
  }

  // Sizing constraints
  if (node.sizingConstraints) {
    const constraintsInfo = formatSizingConstraints(node.sizingConstraints)
    if (constraintsInfo) {
      lines.push(`${prefix}  - Sizing: ${constraintsInfo}`)
    }
  }

  // Visual properties
  if (node.fills && node.fills.length > 0) {
    const fillInfo = formatPaintInfo(node.fills)
    if (fillInfo !== 'none') {
      lines.push(`${prefix}  - Fill: ${fillInfo}`)
    }
  }

  // Stroke/border properties
  if (node.strokeProperties || (node.strokes && node.strokes.length > 0)) {
    const strokeInfo = node.strokeProperties
      ? formatStrokeProperties(node.strokeProperties, node.strokes)
      : formatPaintInfo(node.strokes!)
    if (strokeInfo && strokeInfo !== 'none') {
      lines.push(`${prefix}  - Border: ${strokeInfo}`)
    }
  }

  // Corner/radius properties
  if (node.cornerProperties) {
    const cornerInfo = formatCornerProperties(node.cornerProperties)
    if (cornerInfo) {
      lines.push(`${prefix}  - Corners: ${cornerInfo}`)
    }
  }

  if (node.effects && node.effects.length > 0) {
    const effectsInfo = formatEffectsInfo(node.effects)
    if (effectsInfo) {
      lines.push(`${prefix}  - Effects: ${effectsInfo}`)
    }
  }

  if (node.opacity !== undefined && node.opacity < 1) {
    lines.push(`${prefix}  - Opacity: ${Math.round(node.opacity * 100)}%`)
  }

  // Children
  if (node.children && node.children.length > 0) {
    const visibleChildren = node.children.filter((child) => child.visible !== false)
    if (visibleChildren.length > 0) {
      lines.push(`${prefix}  Children:`)
      for (const child of visibleChildren) {
        lines.push(...formatNode(child, indent + 2))
      }
    }
  }

  return lines
}

/**
 * Generate formatted text context from Figma node metadata
 * Similar to generateComponentList but for Figma designs
 */
export function generateFigmaContextText(
  nodeData: SimplifiedFigmaNode,
  fileName: string,
  nodeName: string,
): string {
  const lines: string[] = []

  // Header
  lines.push(`Figma Design: ${fileName} - ${nodeName}`)
  lines.push('')

  // Format the node tree
  lines.push(...formatNode(nodeData))

  return lines.join('\n')
}
