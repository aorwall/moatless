import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { getFigmaImportRoute, importFigmaNodeRoute } from '@moatless/api-contract/figma'
import { apiClient } from '@/lib/api/client'

type FigmaImportResponse = RouteSuccess<typeof importFigmaNodeRoute>
type FigmaImportMetadata = RouteSuccess<typeof getFigmaImportRoute>

export const figmaApi = {
  async importNode(url: string): Promise<FigmaImportResponse> {
    const path = buildPath(importFigmaNodeRoute)
    return await apiClient.post<FigmaImportResponse>(path, { url })
  },

  async getFigmaImport(importId: string): Promise<FigmaImportMetadata> {
    const path = buildPath(getFigmaImportRoute, { importId })
    return await apiClient.get<FigmaImportMetadata>(path)
  },

  getImageUrl(importId: string): string {
    return `/api/v1/figma/imports/${importId}/image`
  },
}
