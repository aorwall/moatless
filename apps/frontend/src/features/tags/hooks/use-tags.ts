import type { TagListResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { tagsApi } from '@/features/tags/api/tags'
import { queryKeys } from '@/lib/api/query-keys'

export function useTagsList(
  options?: Omit<UseQueryOptions<TagListResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tags.list(),
    queryFn: () => tagsApi.listTags(),
    ...options,
  })
}
