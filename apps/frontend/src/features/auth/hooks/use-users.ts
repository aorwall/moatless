import type { UserListResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import { listUsersHandler } from '@moatless/api-client/sandbox/users/users'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

export function useUsersList(
  options?: Omit<UseQueryOptions<UserListResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.users.list(),
    queryFn: () => unwrapResponse<UserListResponse>(listUsersHandler()),
    ...options,
  })
}
