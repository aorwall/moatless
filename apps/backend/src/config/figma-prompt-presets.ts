import type { FigmaPromptConfig } from '@moatless/types'

/**
 * Preset configurations for Figma prompt generation
 */

/**
 * Original REST API format configuration
 * Matches the original transformer behavior before clipboard support
 * - Uses Figma official property names (not Tailwind)
 * - Includes effects (shadows/blurs)
 * - No design token indicators
 * - No sizing mode indicators
 */
export const originalRestApiPreset: FigmaPromptConfig = {
  sections: {
    layout: true,
    sizing: false, // Not in original
    constraints: false,
    typography: true,
    designTokens: false, // Not in original
    componentProps: false,
    fills: true,
    strokes: true,
    corners: true,
    effects: true, // Only in REST API
    opacity: true,
  },
  propertyStyle: 'figma', // "Horizontal auto-layout, 16px gap"
  maxTextLength: 100,
  showEmptyChildren: true,
}

/**
 * Clipboard format configuration
 * Matches the current clipboard transformer behavior
 * - Uses Tailwind-friendly property names
 * - Includes design token indicators (🎨)
 * - Includes sizing mode indicators (hug/fixed)
 * - No effects (not available in clipboard data)
 */
export const clipboardPreset: FigmaPromptConfig = {
  sections: {
    layout: true,
    sizing: true, // Unique to clipboard
    constraints: false,
    typography: true,
    designTokens: true, // Unique to clipboard
    componentProps: false,
    fills: true,
    strokes: true,
    corners: true,
    effects: false, // Not available in clipboard
    opacity: true,
  },
  propertyStyle: 'tailwind', // "Horizontal, gap-16px"
  maxTextLength: 60, // Shorter truncation for readability
  showEmptyChildren: true,
}

/**
 * Minimal configuration
 * Only essential layout and content information
 * - Layout and typography only
 * - Basic fills for colors
 * - Everything else disabled
 */
export const minimalPreset: FigmaPromptConfig = {
  sections: {
    layout: true,
    sizing: false,
    constraints: false,
    typography: true,
    designTokens: false,
    componentProps: false,
    fills: true,
    strokes: false,
    corners: false,
    effects: false,
    opacity: false,
  },
  propertyStyle: 'tailwind',
  maxTextLength: 50,
  showEmptyChildren: false, // Hide empty nodes
}

/**
 * Detailed configuration
 * Everything enabled for maximum information
 * - All sections included
 * - Tailwind property style
 * - Longer text truncation
 */
export const detailedPreset: FigmaPromptConfig = {
  sections: {
    layout: true,
    sizing: true,
    constraints: true,
    typography: true,
    designTokens: true,
    componentProps: true,
    fills: true,
    strokes: true,
    corners: true,
    effects: true,
    opacity: true,
  },
  propertyStyle: 'tailwind',
  maxTextLength: 150,
  showEmptyChildren: true,
}

/**
 * Get preset configuration by name
 */
export function getPresetConfig(preset: string): FigmaPromptConfig {
  switch (preset) {
    case 'originalRestApi':
      return originalRestApiPreset
    case 'clipboard':
      return clipboardPreset
    case 'minimal':
      return minimalPreset
    case 'detailed':
      return detailedPreset
    default:
      // Default to clipboard preset
      return clipboardPreset
  }
}

/**
 * Merge partial config with a preset
 */
export function mergeWithPreset(
  preset: string,
  overrides?: Partial<FigmaPromptConfig>,
): FigmaPromptConfig {
  const baseConfig = getPresetConfig(preset)

  if (!overrides) {
    return baseConfig
  }

  return {
    ...baseConfig,
    ...overrides,
    sections: {
      ...baseConfig.sections,
      ...overrides.sections,
    },
  }
}
