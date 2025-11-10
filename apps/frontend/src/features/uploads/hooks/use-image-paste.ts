import type { UploadFileResponse } from '@moatless/types'
import type { UseMutationResult } from '@tanstack/react-query'
import type React from 'react'
import { useCallback } from 'react'

/**
 * Hook for handling image paste events in textareas/inputs
 * Automatically uploads pasted images using the provided upload mutation
 */
export function useImagePaste(uploadMutation: UseMutationResult<UploadFileResponse, Error, File>) {
  const handlePaste = useCallback(
    (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const items = e.clipboardData?.items
      if (!items) {
        return
      }

      // Check if any clipboard item is an image
      for (const item of Array.from(items)) {
        if (item.type.startsWith('image/')) {
          // Prevent default paste behavior for images
          e.preventDefault()

          const file = item.getAsFile()
          if (file) {
            // Generate a filename with timestamp
            const extension = file.type.split('/')[1] || 'png'
            const timestamp = Date.now()
            const filename = `pasted-image-${timestamp}.${extension}`

            // Create a new File with the generated name
            const namedFile = new File([file], filename, { type: file.type })

            // Upload the file using the provided upload mutation
            uploadMutation.mutate(namedFile)
          }
        }
      }
    },
    [uploadMutation],
  )

  return handlePaste
}
