import { buildPath, type RouteSuccess } from '@moatless/api-contract'
import { importFigmaNodeRoute } from '@moatless/api-contract/figma'
import { apiClient } from '@/lib/api/client'

type FigmaImportResponse = RouteSuccess<typeof importFigmaNodeRoute>

export const figmaApi = {
  async importNode(url: string): Promise<FigmaImportResponse> {
    const path = buildPath(importFigmaNodeRoute)
    return await apiClient.post<FigmaImportResponse>(path, { url })
  },

  getImageUrl(importId: string): string {
    return `/api/v1/figma/imports/${importId}/image`
  },
}
