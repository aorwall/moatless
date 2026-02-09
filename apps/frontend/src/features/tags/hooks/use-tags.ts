import type { TagResponse } from '@moatless/api-client'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { tagsApi } from '@/features/tags/api/tags'
import { queryKeys } from '@/lib/api/query-keys'

export function useTagsList(
  options?: Omit<UseQueryOptions<TagResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.tags.list(),
    queryFn: () => tagsApi.listTags(),
    ...options,
  })
}
