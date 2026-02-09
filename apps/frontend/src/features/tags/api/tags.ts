import type { TagResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { listTags as listTagsHandler } from '@moatless/api-client/sandbox/tags/tags'

export const tagsApi = {
  async listTags(): Promise<TagResponse[]> {
    return await unwrapResponse(listTagsHandler())
  },
}
