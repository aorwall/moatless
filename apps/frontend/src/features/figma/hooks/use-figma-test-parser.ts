import { type ComponentTree, type TransformOptions, transformClipboard } from '@moatless/figma'
import { useCallback, useState } from 'react'
import rfdc from 'rfdc'
import { figmaApi } from '../api/figma'

const clone = rfdc()

export interface ParsedResult {
  componentTree: ComponentTree
  rawClipboardJson: unknown
  fileName: string
  nodeName: string
  page1Canvas: unknown | null
  internalCanvas: unknown | null
}

export interface UseFigmaTestParserReturn {
  parse: (input: string, options: TransformOptions) => void
  result: ParsedResult | null
  error: string | null
  isLoading: boolean
  clear: () => void
}

/**
 * Helper function to safely extract canvas children from clipboard data
 * Creates deep copies to preserve raw structure before transformation
 */
function extractCanvasChildren(clipboardData: unknown): {
  page1Canvas: unknown | null
  internalCanvas: unknown | null
} {
  // Type guard to check if clipboardData has expected structure
  if (
    clipboardData &&
    typeof clipboardData === 'object' &&
    'root' in clipboardData &&
    clipboardData.root &&
    typeof clipboardData.root === 'object' &&
    'children' in clipboardData.root &&
    Array.isArray(clipboardData.root.children)
  ) {
    const children = clipboardData.root.children
    return {
      // Deep clone children to prevent mutation by transformer (using rfdc for performance)
      page1Canvas: children.length > 0 ? clone(children[0]) : null,
      internalCanvas: children.length > 1 ? clone(children[1]) : null,
    }
  }

  return { page1Canvas: null, internalCanvas: null }
}

/**
 * Hook for parsing Figma clipboard data
 * Uses backend API for HTML parsing, then applies custom transform options
 */
export function useFigmaTestParser(): UseFigmaTestParserReturn {
  const [result, setResult] = useState<ParsedResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  const parse = useCallback(async (input: string, options: TransformOptions) => {
    setIsLoading(true)
    setError(null)
    setResult(null)

    try {
      let clipboardData: unknown

      // Detect HTML clipboard format
      const isHtmlClipboard = input.includes('data-metadata') && input.includes('data-buffer')

      if (isHtmlClipboard) {
        // HTML clipboard - send to backend for parsing
        try {
          const { clipboardJson } = await figmaApi.parseClipboard(input)
          clipboardData = clipboardJson
        } catch (err) {
          const message = err instanceof Error ? err.message : 'Failed to parse HTML clipboard'
          throw new Error(`Backend parsing failed: ${message}`)
        }
      } else {
        // Try to parse as JSON directly
        try {
          clipboardData = JSON.parse(input)
        } catch {
          throw new Error('Invalid input: Not valid JSON or Figma clipboard HTML format')
        }
      }

      // Extract canvas children BEFORE transformation to preserve raw structure
      const { page1Canvas, internalCanvas } = extractCanvasChildren(clipboardData)

      // Apply TransformOptions client-side
      const componentTree = transformClipboard(clipboardData, options)

      // Extract file and node names from metadata
      const fileName = componentTree.source?.fileName || 'Figma Design'
      const nodeName = componentTree.root.name || 'Component'

      setResult({
        componentTree,
        rawClipboardJson: clipboardData,
        fileName,
        nodeName,
        page1Canvas,
        internalCanvas,
      })
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Unknown error occurred'
      setError(message)
    } finally {
      setIsLoading(false)
    }
  }, [])

  const clear = useCallback(() => {
    setResult(null)
    setError(null)
    setIsLoading(false)
  }, [])

  return {
    parse,
    result,
    error,
    isLoading,
    clear,
  }
}
