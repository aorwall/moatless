import { useMutation } from '@tanstack/react-query'
import { useCallback } from 'react'
import { toast } from 'sonner'
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
  const mutation = useMutation({
    mutationFn: figmaApi.importClipboard,
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
      const items = e.clipboardData?.items
      if (!items) {
        return false
      }

      // Check for HTML data (Figma clipboard uses HTML format)
      for (const item of Array.from(items)) {
        if (item.type === 'text/html') {
          // Prevent default paste immediately to avoid text being inserted
          e.preventDefault()

          // Get HTML content asynchronously to check if it's Figma
          item.getAsString((html) => {
            // Check if it's Figma clipboard format
            // Figma clipboard contains data-metadata and data-buffer spans
            if (html.includes('data-metadata') && html.includes('data-buffer')) {
              // This is Figma clipboard data - parse it
              mutation.mutate(html)
            }
            // If not Figma, we've already prevented default paste
            // This is acceptable as we're specifically detecting Figma content
          })

          // Return true to indicate we handled the HTML item
          return true
        }
      }

      return false
    },
    [mutation],
  )

  return handlePaste
}
