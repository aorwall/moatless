import { buildPath } from '@moatless/api-contract'
import { listUsersRoute } from '@moatless/api-contract/users'
import type { UserListResponse } from '@moatless/types'
import { apiClient } from '@/lib/api/client'

export const usersApi = {
  async listUsers(): Promise<UserListResponse> {
    const path = buildPath(listUsersRoute)
    return await apiClient.get<UserListResponse>(path)
  },
}
