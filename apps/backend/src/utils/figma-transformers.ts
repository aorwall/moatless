import type { Effect, Node, Paint } from '@moatless/types'

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
export function formatPaintInfo(paints: Paint[]): string {
  if (!paints || paints.length === 0) {
    return 'none'
  }

  const visiblePaints = paints.filter((p) => p.visible !== false)
  if (visiblePaints.length === 0) {
    return 'none'
  }

  return visiblePaints
    .map((paint) => {
      if (paint.type === 'SOLID' && 'color' in paint && paint.color) {
        return formatColor(paint.color)
      }
      if (paint.type === 'IMAGE') {
        return 'Image fill'
      }
      if (paint.type.startsWith('GRADIENT_')) {
        return `${paint.type.replace('GRADIENT_', '')} gradient`
      }
      if (paint.type === 'PATTERN') {
        return 'Pattern fill'
      }
      return paint.type
    })
    .join(', ')
}

/**
 * Format effects (shadows, blurs) into readable text
 */
export function formatEffectsInfo(effects: Effect[]): string {
  if (!effects || effects.length === 0) {
    return ''
  }

  const visibleEffects = effects.filter((e) => e.visible !== false)
  if (visibleEffects.length === 0) {
    return ''
  }

  return (
    visibleEffects
      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Effect type checking requires checking multiple conditional properties
      .map((effect) => {
        const parts: string[] = []

        if (effect.type === 'DROP_SHADOW') {
          parts.push('Drop shadow')
          if ('radius' in effect && effect.radius) {
            parts.push(`${effect.radius}px blur`)
          }
          if ('color' in effect && effect.color) {
            parts.push(formatColor(effect.color))
          }
        } else if (effect.type === 'INNER_SHADOW') {
          parts.push('Inner shadow')
          if ('radius' in effect && effect.radius) {
            parts.push(`${effect.radius}px blur`)
          }
        } else if (effect.type === 'LAYER_BLUR' || effect.type === 'BACKGROUND_BLUR') {
          const blurType = effect.type === 'LAYER_BLUR' ? 'Layer blur' : 'Background blur'
          if ('radius' in effect) {
            parts.push(`${blurType} ${effect.radius || 0}px`)
          }
        }

        return parts.join(' ')
      })
      .join(', ')
  )
}

/**
 * Format dimensions into readable text
 */
export function formatDimensions(node: Node): string {
  if ('absoluteBoundingBox' in node && node.absoluteBoundingBox) {
    const { width, height } = node.absoluteBoundingBox
    return `${Math.round(width)}x${Math.round(height)}px`
  }
  return ''
}

/**
 * Format a single node into readable text with indentation
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Node formatting requires checking all possible node properties and styling
function formatNode(node: Node, indent = 0): string[] {
  const lines: string[] = []
  const prefix = '  '.repeat(indent)
  const arrow = indent > 0 ? '→ ' : ''

  // Node header with type, name, and dimensions
  const dimensions = formatDimensions(node)
  const header = `${prefix}${arrow}${node.type} "${node.name}"${dimensions ? ` (${dimensions})` : ''}`
  lines.push(header)

  // Layout info (for FRAME nodes)
  if (node.type === 'FRAME' && 'layoutMode' in node) {
    const layoutParts: string[] = []

    if (node.layoutMode === 'HORIZONTAL') {
      layoutParts.push('Horizontal auto-layout')
    } else if (node.layoutMode === 'VERTICAL') {
      layoutParts.push('Vertical auto-layout')
    }

    if ('itemSpacing' in node && node.itemSpacing) {
      layoutParts.push(`${node.itemSpacing}px gap`)
    }

    if (
      'paddingLeft' in node ||
      'paddingRight' in node ||
      'paddingTop' in node ||
      'paddingBottom' in node
    ) {
      const padding = [
        node.paddingTop || 0,
        node.paddingRight || 0,
        node.paddingBottom || 0,
        node.paddingLeft || 0,
      ]
      if (padding.every((p) => p === padding[0])) {
        layoutParts.push(`${padding[0]}px padding`)
      } else {
        layoutParts.push(`padding: ${padding.join('px ')}px`)
      }
    }

    if (layoutParts.length > 0) {
      lines.push(`${prefix}  - Layout: ${layoutParts.join(', ')}`)
    }
  }

  // Typography info (for TEXT nodes)
  if (node.type === 'TEXT' && 'characters' in node) {
    const typoParts: string[] = []

    // Text content (truncated if too long)
    if (node.characters) {
      const truncated =
        node.characters.length > 100 ? `${node.characters.substring(0, 100)}...` : node.characters
      typoParts.push(`"${truncated}"`)
    }

    // Font info from style
    if ('style' in node && node.style) {
      const fontParts: string[] = []
      if ('fontSize' in node.style && node.style.fontSize) {
        fontParts.push(`${node.style.fontSize}px`)
      }
      if ('fontWeight' in node.style && node.style.fontWeight) {
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
        fontParts.push(weightMap[node.style.fontWeight] || `${node.style.fontWeight}`)
      }
      if ('fontFamily' in node.style && node.style.fontFamily) {
        fontParts.push(node.style.fontFamily)
      }

      if (fontParts.length > 0) {
        typoParts.push(`(${fontParts.join(', ')})`)
      }
    }

    if (typoParts.length > 0) {
      lines.push(`${prefix}  - Text: ${typoParts.join(' ')}`)
    }
  }

  // Visual properties - fills
  if ('fills' in node && node.fills && Array.isArray(node.fills) && node.fills.length > 0) {
    const fillInfo = formatPaintInfo(node.fills)
    if (fillInfo !== 'none') {
      lines.push(`${prefix}  - Fill: ${fillInfo}`)
    }
  }

  // Stroke/border properties
  if ('strokes' in node && node.strokes && Array.isArray(node.strokes) && node.strokes.length > 0) {
    const strokeInfo = formatPaintInfo(node.strokes)
    if (strokeInfo !== 'none') {
      const strokeParts: string[] = [strokeInfo]

      if ('strokeWeight' in node && node.strokeWeight) {
        strokeParts.unshift(`${node.strokeWeight}px border`)
      }

      lines.push(`${prefix}  - Border: ${strokeParts.join(', ')}`)
    }
  }

  // Corner/radius properties
  if ('cornerRadius' in node && node.cornerRadius) {
    lines.push(`${prefix}  - Corners: ${node.cornerRadius}px radius`)
  } else if ('rectangleCornerRadii' in node && node.rectangleCornerRadii) {
    const [tl, tr, br, bl] = node.rectangleCornerRadii
    if (tl === tr && tr === br && br === bl) {
      lines.push(`${prefix}  - Corners: ${tl}px radius`)
    } else {
      lines.push(`${prefix}  - Corners: radius (${tl}px ${tr}px ${br}px ${bl}px)`)
    }
  }

  // Effects
  if ('effects' in node && node.effects && Array.isArray(node.effects) && node.effects.length > 0) {
    const effectsInfo = formatEffectsInfo(node.effects)
    if (effectsInfo) {
      lines.push(`${prefix}  - Effects: ${effectsInfo}`)
    }
  }

  // Opacity
  if ('opacity' in node && node.opacity !== undefined && node.opacity < 1) {
    lines.push(`${prefix}  - Opacity: ${Math.round(node.opacity * 100)}%`)
  }

  // Children
  if ('children' in node && node.children && Array.isArray(node.children)) {
    const visibleChildren = node.children.filter(
      (child) => !('visible' in child) || child.visible !== false,
    )
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
 * Accepts raw Figma API node structure (Record<string, unknown> or Node)
 */
export function generateFigmaContextText(
  nodeData: Record<string, unknown> | Node,
  fileName: string,
  nodeName: string,
): string {
  const lines: string[] = []

  // Header
  lines.push(`Figma Design: ${fileName} - ${nodeName}`)
  lines.push('')

  // Format the node tree
  lines.push(...formatNode(nodeData as Node))

  return lines.join('\n')
}
