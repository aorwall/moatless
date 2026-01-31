import { buildPath } from '@moatless/api-contract'
import { listTagsRoute } from '@moatless/api-contract/tags'
import type { TagListResponse } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export const tagsApi = {
  async listTags(): Promise<TagListResponse> {
    const path = buildPath(listTagsRoute)
    return await apiClient.get<TagListResponse>(path)
  },
}
