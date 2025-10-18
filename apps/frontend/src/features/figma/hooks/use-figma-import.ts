import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery } from '@tanstack/react-query'
import { figmaApi } from '../api/figma'

type FigmaImportResponse = Awaited<ReturnType<typeof figmaApi.importNode>>
type FigmaImportMetadata = Awaited<ReturnType<typeof figmaApi.getFigmaImport>>

interface FigmaImportRequest {
  url: string
  includeImageInPrompt?: boolean
}

export function useFigmaImport(
  options?: UseMutationOptions<FigmaImportResponse, Error, FigmaImportRequest>,
) {
  return useMutation({
    mutationFn: ({ url, includeImageInPrompt = false }: FigmaImportRequest) =>
      figmaApi.importNode(url, includeImageInPrompt),
    ...options,
  })
}

export function useFigmaImportQuery(
  importId: string | null | undefined,
  options?: Omit<UseQueryOptions<FigmaImportMetadata, Error>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: ['figma', 'imports', importId],
    queryFn: () => {
      if (!importId) {
        throw new Error('Import ID is required')
      }
      return figmaApi.getFigmaImport(importId)
    },
    enabled: !!importId,
    ...options,
  })
}
