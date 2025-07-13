import type { PreviewConfig } from '@moatless/types/types/repository'
import { type UseMutationOptions, useMutation } from '@tanstack/react-query'
import type { ApiError } from '@/lib/api/client'
import { detectFramework } from '../api/detect-framework'

export function useDetectFramework(
  repositoryId: string,
  options?: UseMutationOptions<Partial<PreviewConfig>, ApiError, void>,
) {
  return useMutation<Partial<PreviewConfig>, ApiError, void>({
    mutationFn: async () => {
      const response = await detectFramework(repositoryId)
      return response.previewConfig
    },
    ...options,
  })
}
