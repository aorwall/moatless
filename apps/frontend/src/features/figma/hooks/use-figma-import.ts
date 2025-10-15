import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery } from '@tanstack/react-query'
import { figmaApi } from '../api/figma'

type FigmaImportResponse = Awaited<ReturnType<typeof figmaApi.importNode>>
type FigmaImportMetadata = Awaited<ReturnType<typeof figmaApi.getFigmaImport>>

export function useFigmaImport(options?: UseMutationOptions<FigmaImportResponse, Error, string>) {
  return useMutation({
    mutationFn: (url: string) => figmaApi.importNode(url),
    ...options,
  })
}

export function useFigmaImportQuery(
  importId: string | null | undefined,
  options?: Omit<UseQueryOptions<FigmaImportMetadata, Error>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: ['figma', 'imports', importId],
    queryFn: () => figmaApi.getFigmaImport(importId!),
    enabled: !!importId,
    ...options,
  })
}
