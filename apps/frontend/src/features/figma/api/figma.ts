import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import {
  getFigmaImportRoute,
  importFigmaClipboardRoute,
  importFigmaNodeRoute,
} from '@moatless/api-contract/figma'
import { apiClient } from '@/lib/api/client'

type FigmaImportResponse = RouteSuccess<typeof importFigmaNodeRoute>
type FigmaImportMetadata = RouteSuccess<typeof getFigmaImportRoute>

export const figmaApi = {
  async importNode(url: string, includeImageInPrompt = false): Promise<FigmaImportResponse> {
    const path = buildPath(importFigmaNodeRoute)
    return await apiClient.post<FigmaImportResponse>(path, { url, includeImageInPrompt })
  },

  async importClipboard(
    clipboardHtml: string,
    includeImageInPrompt = false,
  ): Promise<FigmaImportResponse> {
    const path = buildPath(importFigmaClipboardRoute)
    return await apiClient.post<FigmaImportResponse>(path, { clipboardHtml, includeImageInPrompt })
  },

  async getFigmaImport(importId: string): Promise<FigmaImportMetadata> {
    const path = buildPath(getFigmaImportRoute, { importId })
    return await apiClient.get<FigmaImportMetadata>(path)
  },

  getImageUrl(importId: string): string {
    return `/api/v1/figma/imports/${importId}/image`
  },
}

// Export types for use in hooks
export type { FigmaImportResponse }
