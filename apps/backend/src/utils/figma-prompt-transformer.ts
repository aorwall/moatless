/**
 * Unified Figma Prompt Transformer
 * Supports both clipboard format (fig-kiwie) and REST API format
 * with configurable sections
 */

import type { Effect, FigmaPromptConfig, Node } from '@moatless/types'
import { clipboardPreset } from '../config/figma-prompt-presets.js'

// ===== Type Definitions =====

interface ClipboardNode {
  guid?: string
  type: string
  name: string
  children?: ClipboardNode[]
  properties?: Record<string, unknown>
}

interface NodeSize {
  x: number
  y: number
}

interface StackAlignment {
  stackMode?: string
  stackPrimarySizing?: string | null
  stackCounterSizing?: string | null
  stackSpacing?: number | null
  stackPrimaryAlignItems?: string | null
  stackCounterAlignItems?: string | null
  stackHorizontalPadding?: number | null
  stackVerticalPadding?: number | null
  stackPaddingLeft?: number | null
  stackPaddingRight?: number | null
  stackPaddingTop?: number | null
  stackPaddingBottom?: number | null
}

interface TextProperties {
  textAlignHorizontal?: string | null
  textAutoResize?: string | null
  maxLines?: number | null
  letterSpacing?: { value: number; units: string } | null
}

interface VariableConsumptionMap {
  entries?: Array<{
    variableField?: string
    variableData?: unknown
  }>
}

// ===== Utility Functions =====

/**
 * Format color to hex string
 */
function formatColor(color: { r: number; g: number; b: number; a?: number }): string {
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
 * Format paint information (works with both clipboard and REST API formats)
 */
function formatPaintInfo(paints: unknown): string {
  if (!(paints && Array.isArray(paints)) || paints.length === 0) {
    return ''
  }

  const visiblePaints = paints.filter((p: { visible?: boolean }) => p.visible !== false)
  if (visiblePaints.length === 0) {
    return ''
  }

  return visiblePaints
    .map((paint: { type?: string; color?: { r: number; g: number; b: number; a?: number } }) => {
      if (paint.type === 'SOLID' && paint.color) {
        return formatColor(paint.color)
      }
      if (paint.type === 'IMAGE') {
        return 'Image fill'
      }
      if (paint.type?.startsWith('GRADIENT_')) {
        return `${paint.type.replace('GRADIENT_', '')} gradient`
      }
      if (paint.type === 'PATTERN') {
        return 'Pattern fill'
      }
      return paint.type || ''
    })
    .filter(Boolean)
    .join(', ')
}

/**
 * Format effects (shadows, blurs) - REST API only
 */
function formatEffectsInfo(effects: Effect[]): string {
  if (!effects || effects.length === 0) {
    return ''
  }

  const visibleEffects = effects.filter((e) => e.visible !== false)
  if (visibleEffects.length === 0) {
    return ''
  }

  return (
    visibleEffects
      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Effect formatting requires checking multiple effect types and their optional properties
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
 * Format dimensions
 */
function formatDimensions(node: Node | ClipboardNode, _config: FigmaPromptConfig): string {
  // Try REST API format first
  if ('absoluteBoundingBox' in node && node.absoluteBoundingBox) {
    const { width, height } = node.absoluteBoundingBox
    return `${Math.round(width)}x${Math.round(height)}px`
  }

  // Try clipboard format
  if ('properties' in node && node.properties) {
    const size = node.properties['size'] as NodeSize | undefined
    if (size) {
      return `${Math.round(size.x)}x${Math.round(size.y)}px`
    }
  }

  return ''
}

// ===== Section Formatting Functions =====

/**
 * Format auto-layout properties
 * Supports both clipboard (stackMode) and REST API (layoutMode) formats
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Layout formatting requires checking multiple format types and property combinations
function formatAutoLayout(node: Node | ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.layout) {
    return []
  }

  const lines: string[] = []
  const parts: string[] = []

  // Check clipboard format (properties.stackMode)
  let stackProps: StackAlignment | undefined
  if ('properties' in node && node.properties) {
    stackProps = node.properties as StackAlignment
    if (!stackProps.stackMode) {
      stackProps = undefined
    }
  }

  // Check REST API format (layoutMode)
  let layoutMode: string | undefined
  if ('layoutMode' in node) {
    layoutMode = node.layoutMode as string
  }

  // If neither format has layout, return empty
  if (!(stackProps || layoutMode)) {
    return lines
  }

  // Direction
  const mode = stackProps?.stackMode || layoutMode
  if (mode === 'HORIZONTAL') {
    parts.push(config.propertyStyle === 'tailwind' ? 'Horizontal' : 'Horizontal auto-layout')
  } else if (mode === 'VERTICAL') {
    parts.push(config.propertyStyle === 'tailwind' ? 'Vertical' : 'Vertical auto-layout')
  }

  // Sizing mode (clipboard only)
  if (config.sections.sizing && stackProps) {
    if (stackProps.stackPrimarySizing === 'FIXED') {
      parts.push('fixed')
    } else if (stackProps.stackPrimarySizing === 'AUTO') {
      parts.push('hug')
    }
  }

  // Alignment
  if (stackProps) {
    if (stackProps.stackPrimaryAlignItems) {
      const alignMap: Record<string, string> = {
        MIN: 'justify-start',
        CENTER: 'justify-center',
        MAX: 'justify-end',
        SPACE_BETWEEN: 'justify-between',
      }
      const align = alignMap[stackProps.stackPrimaryAlignItems]
      if (align) {
        parts.push(align)
      }
    }

    if (stackProps.stackCounterAlignItems) {
      const alignMap: Record<string, string> = {
        MIN: 'items-start',
        CENTER: 'items-center',
        MAX: 'items-end',
      }
      const align = alignMap[stackProps.stackCounterAlignItems]
      if (align) {
        parts.push(align)
      }
    }
  }

  // Spacing/gap
  const spacing =
    stackProps?.stackSpacing ?? ('itemSpacing' in node ? (node.itemSpacing as number) : undefined)
  if (spacing !== null && spacing !== undefined) {
    if (config.propertyStyle === 'tailwind') {
      parts.push(`gap-${spacing}px`)
    } else {
      parts.push(`${spacing}px gap`)
    }
  }

  // Padding
  const paddingParts: string[] = []

  if (stackProps) {
    // Clipboard format padding
    const hasIndividualPadding =
      stackProps.stackPaddingLeft !== null ||
      stackProps.stackPaddingRight !== null ||
      stackProps.stackPaddingTop !== null ||
      stackProps.stackPaddingBottom !== null

    if (hasIndividualPadding) {
      const top = (stackProps.stackPaddingTop || 0) as number
      const right = (stackProps.stackPaddingRight || 0) as number
      const bottom = (stackProps.stackPaddingBottom || 0) as number
      const left = (stackProps.stackPaddingLeft || 0) as number

      if (config.propertyStyle === 'tailwind') {
        if (top === right && right === bottom && bottom === left && top > 0) {
          paddingParts.push(`p-${top}px`)
        } else if (top === bottom && left === right) {
          if (top > 0) {
            paddingParts.push(`py-${top}px`)
          }
          if (left > 0) {
            paddingParts.push(`px-${left}px`)
          }
        } else {
          if (top > 0) {
            paddingParts.push(`pt-${top}px`)
          }
          if (right > 0) {
            paddingParts.push(`pr-${right}px`)
          }
          if (bottom > 0) {
            paddingParts.push(`pb-${bottom}px`)
          }
          if (left > 0) {
            paddingParts.push(`pl-${left}px`)
          }
        }
      } else {
        const padding = [Number(top), Number(right), Number(bottom), Number(left)]
        const [pTop = 0] = padding
        if (padding.every((p) => p === pTop)) {
          if (pTop > 0) {
            paddingParts.push(`${pTop}px padding`)
          }
        } else {
          paddingParts.push(`padding: ${padding.join('px ')}px`)
        }
      }
    } else if (
      stackProps.stackHorizontalPadding !== null &&
      stackProps.stackHorizontalPadding !== undefined &&
      stackProps.stackHorizontalPadding > 0
    ) {
      paddingParts.push(`px-${stackProps.stackHorizontalPadding}px`)
    }

    if (
      stackProps.stackVerticalPadding !== null &&
      stackProps.stackVerticalPadding !== undefined &&
      stackProps.stackVerticalPadding > 0
    ) {
      paddingParts.push(`py-${stackProps.stackVerticalPadding}px`)
    }
  } else if (
    'paddingLeft' in node ||
    'paddingRight' in node ||
    'paddingTop' in node ||
    'paddingBottom' in node
  ) {
    // REST API format padding
    const padding = [
      Number(('paddingTop' in node ? (node.paddingTop as number | undefined) : undefined) || 0),
      Number(('paddingRight' in node ? (node.paddingRight as number | undefined) : undefined) || 0),
      Number(
        ('paddingBottom' in node ? (node.paddingBottom as number | undefined) : undefined) || 0,
      ),
      Number(('paddingLeft' in node ? (node.paddingLeft as number | undefined) : undefined) || 0),
    ]
    const [pTop = 0, pRight = 0, pBottom = 0, pLeft = 0] = padding

    if (config.propertyStyle === 'tailwind') {
      if (padding.every((p) => p === pTop) && pTop > 0) {
        paddingParts.push(`p-${pTop}px`)
      } else if (pTop === pBottom && pRight === pLeft) {
        if (pTop > 0) {
          paddingParts.push(`py-${pTop}px`)
        }
        if (pRight > 0) {
          paddingParts.push(`px-${pRight}px`)
        }
      } else {
        if (pTop > 0) {
          paddingParts.push(`pt-${pTop}px`)
        }
        if (pRight > 0) {
          paddingParts.push(`pr-${pRight}px`)
        }
        if (pBottom > 0) {
          paddingParts.push(`pb-${pBottom}px`)
        }
        if (pLeft > 0) {
          paddingParts.push(`pl-${pLeft}px`)
        }
      }
    } else if (padding.every((p) => p === pTop)) {
      if (pTop > 0) {
        paddingParts.push(`${pTop}px padding`)
      }
    } else {
      paddingParts.push(`padding: ${padding.join('px ')}px`)
    }
  }

  if (paddingParts.length > 0) {
    parts.push(...paddingParts)
  }

  if (parts.length > 0) {
    lines.push(`  - Layout: ${parts.join(', ')}`)
  }

  return lines
}

/**
 * Format sizing mode (clipboard only)
 */
function formatSizing(node: ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.sizing) {
    return []
  }

  const lines: string[] = []

  if ('properties' in node && node.properties) {
    const props = node.properties as StackAlignment
    if (props.stackMode && props.stackCounterSizing) {
      const sizingMap: Record<string, string> = {
        FIXED: 'Fixed',
        RESIZE_TO_FIT: 'Hug',
        RESIZE_TO_FIT_WITH_IMPLICIT_SIZE: 'Hug',
      }
      const sizing = sizingMap[props.stackCounterSizing as string]
      if (sizing) {
        lines.push(`  - Sizing: ${sizing} cross-axis`)
      }
    }
  }

  return lines
}

/**
 * Format constraints
 */
function formatConstraints(node: ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.constraints) {
    return []
  }

  const lines: string[] = []

  if ('properties' in node && node.properties) {
    const horizontalConstraint = node.properties['horizontalConstraint'] as string | undefined
    const verticalConstraint = node.properties['verticalConstraint'] as string | undefined

    if (
      horizontalConstraint &&
      verticalConstraint &&
      (horizontalConstraint !== 'CENTER' || verticalConstraint !== 'CENTER')
    ) {
      const constraintMap: Record<string, string> = {
        LEFT: 'left',
        RIGHT: 'right',
        CENTER: 'center',
        SCALE: 'scale',
        LEFT_RIGHT: 'stretch-x',
        STRETCH: 'stretch',
        TOP: 'top',
        BOTTOM: 'bottom',
        TOP_BOTTOM: 'stretch-y',
      }

      const h = constraintMap[horizontalConstraint] || horizontalConstraint.toLowerCase()
      const v = constraintMap[verticalConstraint] || verticalConstraint.toLowerCase()

      lines.push(`  - Constraints: ${h}-x, ${v}-y`)
    }
  }

  return lines
}

/**
 * Format typography properties
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Typography formatting requires checking both clipboard and REST API formats with multiple optional properties
function formatTypography(node: Node | ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.typography || node.type !== 'TEXT') {
    return []
  }

  const lines: string[] = []
  const parts: string[] = []

  // Check clipboard format
  if ('properties' in node && node.properties) {
    const props = node.properties as TextProperties

    // Text alignment
    if (props.textAlignHorizontal) {
      const alignMap: Record<string, string> = {
        LEFT: 'text-left',
        CENTER: 'text-center',
        RIGHT: 'text-right',
        JUSTIFIED: 'text-justify',
      }
      const align = alignMap[props.textAlignHorizontal]
      if (align) {
        parts.push(align)
      }
    }

    // Auto-resize / truncation
    if (props.textAutoResize === 'TRUNCATE' || props.maxLines !== null) {
      if (props.maxLines) {
        parts.push(`line-clamp-${props.maxLines}`)
      } else {
        parts.push('truncate')
      }
    }

    // Letter spacing
    if (props.letterSpacing && props.letterSpacing.value !== 0) {
      if (props.letterSpacing.value > 0) {
        parts.push('tracking-wide')
      } else {
        parts.push('tracking-tight')
      }
    }

    if (parts.length > 0) {
      lines.push(`  - Typography: ${parts.join(', ')}`)
    }

    // Text content
    const textData = node.properties['textData'] as { characters?: string } | undefined
    if (textData?.characters) {
      const maxLength = config.maxTextLength || 60
      const truncated =
        textData.characters.length > maxLength
          ? `${textData.characters.substring(0, maxLength)}...`
          : textData.characters
      lines.push(`  - Content: "${truncated}"`)
    }

    // Font info
    const fontName = node.properties['fontName'] as { family?: string; style?: string } | undefined
    const fontSize = node.properties['fontSize'] as number | undefined
    if (fontName?.family && fontSize) {
      lines.push(`  - Font: ${fontSize}px ${fontName.style || ''} ${fontName.family}`)
    }
  }
  // Check REST API format
  else if ('characters' in node) {
    const maxLength = config.maxTextLength || 100
    const typoParts: string[] = []

    // Text content
    if (node.characters) {
      const truncated =
        node.characters.length > maxLength
          ? `${node.characters.substring(0, maxLength)}...`
          : node.characters
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
      lines.push(`  - Text: ${typoParts.join(' ')}`)
    }
  }

  return lines
}

/**
 * Format design token usage (clipboard only)
 */
function formatDesignTokens(node: ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.designTokens) {
    return []
  }

  const lines: string[] = []

  if ('properties' in node && node.properties) {
    const variableConsumptionMap = node.properties['variableConsumptionMap'] as
      | VariableConsumptionMap
      | undefined

    if (!variableConsumptionMap?.entries || variableConsumptionMap.entries.length === 0) {
      return lines
    }

    const tokenFields = new Set<string>()

    for (const entry of variableConsumptionMap.entries) {
      if (entry.variableField) {
        const fieldMap: Record<string, string> = {
          FILL_COLOR: 'fill',
          STROKE_COLOR: 'stroke',
          STACK_PADDING_LEFT: 'spacing',
          STACK_PADDING_RIGHT: 'spacing',
          STACK_PADDING_TOP: 'spacing',
          STACK_PADDING_BOTTOM: 'spacing',
          WIDTH: 'sizing',
          HEIGHT: 'sizing',
          RECTANGLE_TOP_LEFT_CORNER_RADIUS: 'radius',
          RECTANGLE_TOP_RIGHT_CORNER_RADIUS: 'radius',
          RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS: 'radius',
          RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS: 'radius',
        }

        const friendlyName = fieldMap[entry.variableField] || entry.variableField.toLowerCase()
        tokenFields.add(friendlyName)
      }
    }

    if (tokenFields.size > 0) {
      lines.push(`  - 🎨 Design tokens: ${Array.from(tokenFields).join(', ')}`)
    }
  }

  return lines
}

/**
 * Format component properties (clipboard only)
 */
function formatComponentProps(node: ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.componentProps) {
    return []
  }

  const lines: string[] = []

  if ('properties' in node && node.properties) {
    const componentPropDefs = node.properties['componentPropDefs'] as unknown

    if (
      !(componentPropDefs && Array.isArray(componentPropDefs)) ||
      componentPropDefs.length === 0
    ) {
      return lines
    }

    const props = componentPropDefs
      .map((prop: { name?: string; type?: string; initialValue?: unknown }) => {
        if (!(prop.name && prop.type)) {
          return null
        }

        if (prop.type === 'BOOL' || prop.type === 'TEXT' || prop.type === 'INSTANCE_SWAP') {
          return `${prop.name} (${prop.type.toLowerCase()})`
        }

        return null
      })
      .filter(Boolean)

    if (props.length > 0) {
      lines.push(`  - Component props: ${props.join(', ')}`)
    }
  }

  return lines
}

/**
 * Format fill colors
 */
function formatFills(node: Node | ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.fills) {
    return []
  }

  const lines: string[] = []

  // Try clipboard format
  if ('properties' in node && node.properties) {
    const fillPaints = node.properties['fillPaints'] as unknown
    if (fillPaints) {
      const fillInfo = formatPaintInfo(fillPaints)
      if (fillInfo) {
        lines.push(`  - Fill: ${fillInfo}`)
      }
    }
  }
  // Try REST API format
  else if ('fills' in node && node.fills && Array.isArray(node.fills) && node.fills.length > 0) {
    const fillInfo = formatPaintInfo(node.fills)
    if (fillInfo) {
      lines.push(`  - Fill: ${fillInfo}`)
    }
  }

  return lines
}

/**
 * Format strokes/borders
 */
function formatStrokes(node: Node | ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.strokes) {
    return []
  }

  const lines: string[] = []

  // Try clipboard format
  if ('properties' in node && node.properties) {
    const strokePaints = node.properties['strokePaints'] as unknown
    if (strokePaints) {
      const strokeInfo = formatPaintInfo(strokePaints)
      if (strokeInfo) {
        const strokeWeight = node.properties['strokeWeight'] as number | undefined
        const strokePrefix = strokeWeight ? `${strokeWeight}px border, ` : ''
        lines.push(`  - Border: ${strokePrefix}${strokeInfo}`)
      }
    }
  }
  // Try REST API format
  else if (
    'strokes' in node &&
    node.strokes &&
    Array.isArray(node.strokes) &&
    node.strokes.length > 0
  ) {
    const strokeInfo = formatPaintInfo(node.strokes)
    if (strokeInfo) {
      const strokeParts: string[] = [strokeInfo]

      if ('strokeWeight' in node && node.strokeWeight) {
        strokeParts.unshift(`${node.strokeWeight}px border`)
      }

      lines.push(`  - Border: ${strokeParts.join(', ')}`)
    }
  }

  return lines
}

/**
 * Format corner radius
 */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Corner formatting requires checking multiple corner properties and format variations
function formatCorners(node: Node | ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.corners) {
    return []
  }

  const lines: string[] = []

  // Try clipboard format
  if ('properties' in node && node.properties) {
    const cornerRadius = node.properties['cornerRadius'] as number | undefined
    if (cornerRadius !== undefined && cornerRadius > 0) {
      lines.push(`  - Corners: ${cornerRadius}px radius`)
    } else {
      const topLeft = node.properties['rectangleTopLeftCornerRadius'] as number | undefined
      const topRight = node.properties['rectangleTopRightCornerRadius'] as number | undefined
      const bottomRight = node.properties['rectangleBottomRightCornerRadius'] as number | undefined
      const bottomLeft = node.properties['rectangleBottomLeftCornerRadius'] as number | undefined

      if (topLeft || topRight || bottomRight || bottomLeft) {
        if (topLeft === topRight && topRight === bottomRight && bottomRight === bottomLeft) {
          lines.push(`  - Corners: ${topLeft}px radius`)
        } else {
          lines.push(`  - Corners: ${topLeft}px ${topRight}px ${bottomRight}px ${bottomLeft}px`)
        }
      }
    }
  }
  // Try REST API format
  else if ('cornerRadius' in node && node.cornerRadius) {
    lines.push(`  - Corners: ${node.cornerRadius}px radius`)
  } else if ('rectangleCornerRadii' in node && node.rectangleCornerRadii) {
    const [tl, tr, br, bl] = node.rectangleCornerRadii
    if (tl === tr && tr === br && br === bl) {
      lines.push(`  - Corners: ${tl}px radius`)
    } else {
      lines.push(`  - Corners: radius (${tl}px ${tr}px ${br}px ${bl}px)`)
    }
  }

  return lines
}

/**
 * Format effects (REST API only)
 */
function formatEffects(node: Node, config: FigmaPromptConfig): string[] {
  if (!config.sections.effects) {
    return []
  }

  const lines: string[] = []

  if ('effects' in node && node.effects && Array.isArray(node.effects) && node.effects.length > 0) {
    const effectsInfo = formatEffectsInfo(node.effects)
    if (effectsInfo) {
      lines.push(`  - Effects: ${effectsInfo}`)
    }
  }

  return lines
}

/**
 * Format opacity
 */
function formatOpacity(node: Node | ClipboardNode, config: FigmaPromptConfig): string[] {
  if (!config.sections.opacity) {
    return []
  }

  const lines: string[] = []

  // Try clipboard format
  if ('properties' in node && node.properties) {
    const opacity = node.properties['opacity'] as number | undefined
    if (opacity !== undefined && opacity < 1) {
      lines.push(`  - Opacity: ${Math.round(opacity * 100)}%`)
    }
  }
  // Try REST API format
  else if ('opacity' in node && node.opacity !== undefined && node.opacity < 1) {
    lines.push(`  - Opacity: ${Math.round(node.opacity * 100)}%`)
  }

  return lines
}

// ===== Main Formatting Function =====

/**
 * Format a single node with all configured sections
 */
function formatNode(node: Node | ClipboardNode, config: FigmaPromptConfig, indent = 0): string[] {
  const lines: string[] = []
  const prefix = '  '.repeat(indent)
  const arrow = indent > 0 ? '→ ' : ''

  // Node header with type, name, and dimensions
  const dimensions = formatDimensions(node, config)
  const header = `${prefix}${arrow}${node.type} "${node.name}"${dimensions ? ` (${dimensions})` : ''}`
  lines.push(header)

  // Format all sections based on config
  lines.push(...formatAutoLayout(node, config).map((line) => `${prefix}${line}`))

  // Clipboard-only sections
  if ('properties' in node) {
    lines.push(...formatSizing(node, config).map((line) => `${prefix}${line}`))
    lines.push(...formatConstraints(node, config).map((line) => `${prefix}${line}`))
  }

  lines.push(...formatTypography(node, config).map((line) => `${prefix}${line}`))

  // Clipboard-only design tokens
  if ('properties' in node) {
    lines.push(...formatDesignTokens(node, config).map((line) => `${prefix}${line}`))
    lines.push(...formatComponentProps(node, config).map((line) => `${prefix}${line}`))
  }

  lines.push(...formatFills(node, config).map((line) => `${prefix}${line}`))
  lines.push(...formatStrokes(node, config).map((line) => `${prefix}${line}`))
  lines.push(...formatCorners(node, config).map((line) => `${prefix}${line}`))

  // REST API-only effects
  if (!('properties' in node)) {
    lines.push(...formatEffects(node as Node, config).map((line) => `${prefix}${line}`))
  }

  lines.push(...formatOpacity(node, config).map((line) => `${prefix}${line}`))

  // Children
  if ('children' in node && node.children && Array.isArray(node.children)) {
    const visibleChildren = node.children.filter((child) => {
      // Skip hidden nodes
      if ('properties' in child && child.properties?.['visible'] === false) {
        return false
      }
      if ('visible' in child && child.visible === false) {
        return false
      }

      // Skip variable/token definition nodes (clipboard only)
      if (child.type === 'VARIABLE' || child.type === 'VARIABLE_SET') {
        return false
      }

      // Skip empty children if configured
      if (!config.showEmptyChildren) {
        const hasChildren = 'children' in child && child.children && child.children.length > 0
        if (!hasChildren) {
          return false
        }
      }

      return true
    })

    if (visibleChildren.length > 0) {
      lines.push(`${prefix}  Children:`)
      for (const child of visibleChildren) {
        lines.push(...formatNode(child, config, indent + 2))
      }
    }
  }

  return lines
}

/**
 * Generate formatted text context from Figma node
 * Supports both clipboard and REST API formats
 */
export function generateFigmaPrompt(
  nodeData: Record<string, unknown> | Node | ClipboardNode,
  fileName: string,
  nodeName: string,
  config: FigmaPromptConfig = clipboardPreset,
): string {
  const lines: string[] = []

  // Header
  lines.push(`Figma Design: ${fileName} - ${nodeName}`)
  lines.push('')

  // Format the node tree
  lines.push(...formatNode(nodeData as Node | ClipboardNode, config))

  return lines.join('\n')
}
