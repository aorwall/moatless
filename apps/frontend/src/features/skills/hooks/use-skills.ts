import type { SkillResponse } from '@moatless/api-client'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { skillsApi } from '@/features/skills/api/skills'
import { queryKeys } from '@/lib/api/query-keys'

export function useRepositorySkills(
  repositoryId: string | undefined,
  options?: Omit<UseQueryOptions<SkillResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.skillsForRepository(repositoryId ?? ''),
    queryFn: () => {
      if (!repositoryId) {
        return Promise.resolve([])
      }
      return skillsApi.listRepositorySkills(repositoryId)
    },
    enabled: !!repositoryId,
    // Server caches for 1 hour; frontend can be less aggressive.
    staleTime: 5 * 60 * 1000,
    // Auth/git failures should not retry-storm.
    retry: false,
    ...options,
  })
}
