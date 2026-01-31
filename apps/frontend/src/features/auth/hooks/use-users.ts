import type { UserListResponse } from '@moatless/types'
import type { UseQueryOptions } from '@tanstack/react-query'
import { useQuery } from '@tanstack/react-query'
import { usersApi } from '@/features/auth/api/users'
import { queryKeys } from '@/lib/api/query-keys'

export function useUsersList(
  options?: Omit<UseQueryOptions<UserListResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.users.list(),
    queryFn: () => usersApi.listUsers(),
    ...options,
  })
}
