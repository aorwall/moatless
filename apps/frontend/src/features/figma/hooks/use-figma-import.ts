import type { UseMutationOptions } from '@tanstack/react-query'
import { useMutation } from '@tanstack/react-query'
import { figmaApi } from '../api/figma'

type FigmaImportResponse = Awaited<ReturnType<typeof figmaApi.importNode>>

export function useFigmaImport(options?: UseMutationOptions<FigmaImportResponse, Error, string>) {
  return useMutation({
    mutationFn: (url: string) => figmaApi.importNode(url),
    ...options,
  })
}
