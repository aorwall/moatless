import { useMutation } from '@tanstack/react-query'
import type React from 'react'
import { useCallback } from 'react'
import { toast } from 'sonner'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { getErrorMessage } from '@/lib/utils/error'
import { type FigmaImportResponse, figmaApi } from '../api/figma'

export interface UseFigmaClipboardPasteOptions {
  onSuccess?: (result: FigmaImportResponse) => void
  onError?: (error: Error) => void
}

/**
 * Hook for handling Figma clipboard paste events
 * Automatically detects and parses Figma clipboard data (fig-kiwie format)
 *
 * @example
 * const handlePaste = useFigmaClipboardPaste({
 *   onSuccess: (result) => {
 *     console.log('Parsed Figma design:', result.root.name)
 *     // Add to context items
 *   }
 * })
 *
 * // In your component:
 * <textarea onPaste={handlePaste} />
 */
export function useFigmaClipboardPaste(options?: UseFigmaClipboardPasteOptions) {
  const isFigmaClipboardEnabled = useFeatureFlag('FIGMA_CLIPBOARD_PASTE')

  const mutation = useMutation({
    mutationFn: (clipboardHtml: string) => figmaApi.importClipboard(clipboardHtml, false),
    onSuccess: options?.onSuccess,
    onError:
      options?.onError ||
      ((error) => {
        toast.error('Failed to parse Figma clipboard', {
          description: getErrorMessage(error),
        })
      }),
  })

  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>): boolean => {
      // Feature flag check: if disabled, don't intercept paste
      if (!isFigmaClipboardEnabled) {
        return false
      }

      const clipboardData = e.clipboardData
      if (!clipboardData) {
        return false
      }

      // Check if clipboard has HTML content
      if (clipboardData.types.includes('text/html')) {
        // Get HTML content synchronously to check if it's Figma
        const html = clipboardData.getData('text/html')

        // Check if it's Figma clipboard format
        // Figma clipboard contains data-metadata and data-buffer spans
        if (html.includes('data-metadata') && html.includes('data-buffer')) {
          // This is Figma clipboard data - prevent default and parse it
          e.preventDefault()
          mutation.mutate(html)
          return true
        }
      }

      // Not Figma content - allow normal paste behavior
      return false
    },
    [isFigmaClipboardEnabled, mutation],
  )

  return handlePaste
}
