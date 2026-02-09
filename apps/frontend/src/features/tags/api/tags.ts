import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { listTags as listTagsHandler } from '@moatless/api-client/sandbox/tags/tags'
import type { Tag, TagListResponse } from '@moatless/types'

export const tagsApi = {
  async listTags(): Promise<TagListResponse> {
    const tags = (await unwrapResponse(listTagsHandler())) as unknown as Tag[]
    return { tags }
  },
}
