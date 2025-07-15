import type { PreviewConfig } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export interface DetectFrameworkResponse {
  previewConfig: Partial<PreviewConfig>
}

export async function detectFramework(repositoryId: string): Promise<DetectFrameworkResponse> {
  return apiClient.post<DetectFrameworkResponse>(
    `/api/v1/repositories/${repositoryId}/detect-framework`,
  )
}
