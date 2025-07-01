import { useMutation } from '@tanstack/react-query'
import type { PreviewConfig } from '@moatless/shared/types/repository'
import { detectFramework } from '../api/detect-framework'

export function useDetectFramework(repositoryId: string) {
  return useMutation<Partial<PreviewConfig>, Error>({
    mutationFn: async () => {
      const response = await detectFramework(repositoryId)
      return response.previewConfig
    },
  })
}