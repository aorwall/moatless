import type {
  FigmaConstraints,
  FigmaEffect,
  FigmaLayoutProperties,
  FigmaPaint,
  FigmaRectangle,
  FigmaTypography,
  RawFigmaNode,
  SimplifiedFigmaNode,
} from '@moatless/types'
import { logger } from '../utils/logger.js'

/**
 * Parse raw Figma node into a simplified structure
 * Extracts only the most relevant properties for AI coding contexts
 */
export function parseNode(rawNode: RawFigmaNode): SimplifiedFigmaNode {
  try {
    const simplified: SimplifiedFigmaNode = {
      id: rawNode.id,
      name: rawNode.name,
      type: rawNode.type as SimplifiedFigmaNode['type'],
      visible: rawNode.visible ?? true,
    }

    // Extract bounds and sizing
    if ('absoluteBoundingBox' in rawNode) {
      simplified.absoluteBoundingBox = extractBounds(rawNode['absoluteBoundingBox'])
      if (simplified.absoluteBoundingBox) {
        simplified.size = {
          x: simplified.absoluteBoundingBox.width,
          y: simplified.absoluteBoundingBox.height,
        }
      }
    }

    // Extract layout properties (auto-layout)
    const layout = extractLayoutProperties(rawNode)
    if (layout) {
      simplified.layout = layout
    }

    // Extract visual styling
    if ('fills' in rawNode) {
      simplified.fills = extractPaints(rawNode['fills'] as unknown[])
    }

    if ('strokes' in rawNode) {
      simplified.strokes = extractPaints(rawNode['strokes'] as unknown[])
    }

    if ('effects' in rawNode) {
      simplified.effects = extractEffects(rawNode['effects'] as unknown[])
    }

    if ('opacity' in rawNode && typeof rawNode['opacity'] === 'number') {
      simplified.opacity = rawNode['opacity']
    }

    if ('blendMode' in rawNode) {
      simplified.blendMode = rawNode['blendMode'] as SimplifiedFigmaNode['blendMode']
    }

    if ('isMask' in rawNode) {
      simplified.isMask = rawNode['isMask'] as boolean
    }

    // Extract typography for TEXT nodes
    if (rawNode.type === 'TEXT') {
      const typography = extractTypography(rawNode)
      if (typography) {
        simplified.typography = typography
      }
    }

    // Extract constraints
    if ('constraints' in rawNode) {
      simplified.constraints = extractConstraints(rawNode['constraints'])
    }

    // Recursively parse children
    if (rawNode.children && Array.isArray(rawNode.children)) {
      simplified.children = rawNode.children.map((child) => parseNode(child as RawFigmaNode))
    }

    return simplified
  } catch (error) {
    logger.error('Error parsing Figma node', error as Error, { nodeId: rawNode.id })
    // Return minimal valid node on error
    return {
      id: rawNode.id,
      name: rawNode.name,
      type: rawNode.type as SimplifiedFigmaNode['type'],
      visible: true,
    }
  }
}

/**
 * Extract bounding box properties
 */
function extractBounds(bounds: unknown): FigmaRectangle | undefined {
  if (!bounds || typeof bounds !== 'object') {
    return undefined
  }

  const b = bounds as Record<string, unknown>
  if (
    typeof b['x'] === 'number' &&
    typeof b['y'] === 'number' &&
    typeof b['width'] === 'number' &&
    typeof b['height'] === 'number'
  ) {
    return {
      x: b['x'],
      y: b['y'],
      width: b['width'],
      height: b['height'],
    }
  }

  return undefined
}

/**
 * Extract auto-layout properties
 */
function extractLayoutProperties(node: Record<string, unknown>): FigmaLayoutProperties | undefined {
  const layout: Partial<FigmaLayoutProperties> = {}
  let hasLayoutProps = false

  // Layout mode
  if (node['layoutMode'] && typeof node['layoutMode'] === 'string') {
    layout.layoutMode = node['layoutMode'] as FigmaLayoutProperties['layoutMode']
    hasLayoutProps = true
  }

  // Sizing modes
  if (node['primaryAxisSizingMode'] && typeof node['primaryAxisSizingMode'] === 'string') {
    layout.primaryAxisSizingMode = node[
      'primaryAxisSizingMode'
    ] as FigmaLayoutProperties['primaryAxisSizingMode']
    hasLayoutProps = true
  }

  if (node['counterAxisSizingMode'] && typeof node['counterAxisSizingMode'] === 'string') {
    layout.counterAxisSizingMode = node[
      'counterAxisSizingMode'
    ] as FigmaLayoutProperties['counterAxisSizingMode']
    hasLayoutProps = true
  }

  // Padding
  if (typeof node['paddingLeft'] === 'number') {
    layout.paddingLeft = node['paddingLeft']
  }
  if (typeof node['paddingRight'] === 'number') {
    layout.paddingRight = node['paddingRight']
  }
  if (typeof node['paddingTop'] === 'number') {
    layout.paddingTop = node['paddingTop']
  }
  if (typeof node['paddingBottom'] === 'number') {
    layout.paddingBottom = node['paddingBottom']
  }

  // Spacing
  if (typeof node['itemSpacing'] === 'number') {
    layout.itemSpacing = node['itemSpacing']
    hasLayoutProps = true
  }

  if (typeof node['counterAxisSpacing'] === 'number') {
    layout.counterAxisSpacing = node['counterAxisSpacing']
  }

  // Alignment
  if (node['layoutAlign'] && typeof node['layoutAlign'] === 'string') {
    layout.layoutAlign = node['layoutAlign'] as FigmaLayoutProperties['layoutAlign']
  }

  if (typeof node['layoutGrow'] === 'number') {
    layout.layoutGrow = node['layoutGrow']
  }

  return hasLayoutProps ? (layout as FigmaLayoutProperties) : undefined
}

/**
 * Extract paint properties (fills, strokes)
 */
function extractPaints(paints: unknown[]): FigmaPaint[] | undefined {
  if (!Array.isArray(paints) || paints.length === 0) {
    return undefined
  }

  const parsed = paints
    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Property extraction requires multiple conditional checks for type safety
    .map((paint) => {
      if (!paint || typeof paint !== 'object') {
        return null
      }

      const p = paint as Record<string, unknown>
      if (!p['type'] || typeof p['type'] !== 'string') {
        return null
      }

      const parsed: Partial<FigmaPaint> = {
        type: p['type'] as FigmaPaint['type'],
      }

      if (typeof p['visible'] === 'boolean') {
        parsed.visible = p['visible']
      }
      if (typeof p['opacity'] === 'number') {
        parsed.opacity = p['opacity']
      }

      // Extract color if present
      if (p['color'] && typeof p['color'] === 'object') {
        const c = p['color'] as Record<string, unknown>
        if (
          typeof c['r'] === 'number' &&
          typeof c['g'] === 'number' &&
          typeof c['b'] === 'number'
        ) {
          parsed.color = {
            r: c['r'],
            g: c['g'],
            b: c['b'],
            a: typeof c['a'] === 'number' ? c['a'] : 1,
          }
        }
      }

      if (p['blendMode'] && typeof p['blendMode'] === 'string') {
        parsed.blendMode = p['blendMode'] as FigmaPaint['blendMode']
      }

      return parsed as FigmaPaint
    })
    .filter((p): p is FigmaPaint => p !== null)

  return parsed.length > 0 ? parsed : undefined
}

/**
 * Extract effect properties
 */
function extractEffects(effects: unknown[]): FigmaEffect[] | undefined {
  if (!Array.isArray(effects) || effects.length === 0) {
    return undefined
  }

  const parsed = effects
    // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Property extraction requires multiple conditional checks for type safety
    .map((effect) => {
      if (!effect || typeof effect !== 'object') {
        return null
      }

      const e = effect as Record<string, unknown>
      if (!e['type'] || typeof e['type'] !== 'string') {
        return null
      }

      const parsed: Partial<FigmaEffect> = {
        type: e['type'] as FigmaEffect['type'],
      }

      if (typeof e['visible'] === 'boolean') {
        parsed.visible = e['visible']
      }
      if (typeof e['radius'] === 'number') {
        parsed.radius = e['radius']
      }

      // Extract color
      if (e['color'] && typeof e['color'] === 'object') {
        const c = e['color'] as Record<string, unknown>
        if (
          typeof c['r'] === 'number' &&
          typeof c['g'] === 'number' &&
          typeof c['b'] === 'number'
        ) {
          parsed.color = {
            r: c['r'],
            g: c['g'],
            b: c['b'],
            a: typeof c['a'] === 'number' ? c['a'] : 1,
          }
        }
      }

      // Extract offset
      if (e['offset'] && typeof e['offset'] === 'object') {
        const o = e['offset'] as Record<string, unknown>
        if (typeof o['x'] === 'number' && typeof o['y'] === 'number') {
          parsed.offset = { x: o['x'], y: o['y'] }
        }
      }

      if (typeof e['spread'] === 'number') {
        parsed.spread = e['spread']
      }

      if (e['blendMode'] && typeof e['blendMode'] === 'string') {
        parsed.blendMode = e['blendMode'] as FigmaEffect['blendMode']
      }

      return parsed as FigmaEffect
    })
    .filter((e): e is FigmaEffect => e !== null)

  return parsed.length > 0 ? parsed : undefined
}

/**
 * Extract typography properties for TEXT nodes
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Typography extraction requires checking multiple style properties
function extractTypography(node: Record<string, unknown>): FigmaTypography | undefined {
  const typo: Partial<FigmaTypography> = {}
  let hasTypoProps = false

  if (node['style'] && typeof node['style'] === 'object') {
    const style = node['style'] as Record<string, unknown>

    if (typeof style['fontFamily'] === 'string') {
      typo.fontFamily = style['fontFamily']
      hasTypoProps = true
    }

    if (typeof style['fontPostScriptName'] === 'string') {
      typo.fontPostScriptName = style['fontPostScriptName']
    }

    if (typeof style['fontSize'] === 'number') {
      typo.fontSize = style['fontSize']
      hasTypoProps = true
    }

    if (typeof style['fontWeight'] === 'number') {
      typo.fontWeight = style['fontWeight']
    }

    if (typeof style['textAlignHorizontal'] === 'string') {
      typo.textAlignHorizontal = style[
        'textAlignHorizontal'
      ] as FigmaTypography['textAlignHorizontal']
    }

    if (typeof style['textAlignVertical'] === 'string') {
      typo.textAlignVertical = style['textAlignVertical'] as FigmaTypography['textAlignVertical']
    }

    if (typeof style['letterSpacing'] === 'number') {
      typo.letterSpacing = style['letterSpacing']
    }

    if (typeof style['lineHeightPx'] === 'number') {
      typo.lineHeightPx = style['lineHeightPx']
    }

    if (typeof style['lineHeightPercent'] === 'number') {
      typo.lineHeightPercent = style['lineHeightPercent']
    }
  }

  if (typeof node['characters'] === 'string') {
    typo.characters = node['characters']
    hasTypoProps = true
  }

  return hasTypoProps ? (typo as FigmaTypography) : undefined
}

/**
 * Extract constraint properties
 */
function extractConstraints(constraints: unknown): FigmaConstraints | undefined {
  if (!constraints || typeof constraints !== 'object') {
    return undefined
  }

  const c = constraints as Record<string, unknown>
  const parsed: Partial<FigmaConstraints> = {}

  if (typeof c['vertical'] === 'string') {
    parsed.vertical = c['vertical'] as FigmaConstraints['vertical']
  }

  if (typeof c['horizontal'] === 'string') {
    parsed.horizontal = c['horizontal'] as FigmaConstraints['horizontal']
  }

  return Object.keys(parsed).length > 0 ? (parsed as FigmaConstraints) : undefined
}
