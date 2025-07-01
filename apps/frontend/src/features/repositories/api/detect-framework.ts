import type { PreviewConfig } from '@moatless/shared/types/repository'
import { apiClient } from '@/lib/api/client'

export interface DetectFrameworkResponse {
  previewConfig: Partial<PreviewConfig>
}

export async function detectFramework(repositoryId: string): Promise<DetectFrameworkResponse> {
  return apiClient.post<DetectFrameworkResponse>(`/repositories/${repositoryId}/detect-framework`)
}