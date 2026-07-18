import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  addMember,
  createTeam,
  listMembers,
  listTeams,
  removeMember,
  updateTeam,
} from '@moatless/api-client/sandbox/teams/teams'
import { listUsersHandler, updateUserHandler } from '@moatless/api-client/sandbox/users/users'
import { useMutation, useQueries, useQuery, useQueryClient } from '@tanstack/react-query'
import { queryKeys } from '@/lib/api/query-keys'

export type UserRoleValue = 'user' | 'admin'

type ListUsersHandlerResponse = Awaited<ReturnType<typeof listUsersHandler>>
type ListUsersHandlerSuccess = Extract<ListUsersHandlerResponse, { status: 200 }>
type AdminUserListResponse = ListUsersHandlerSuccess['data']
export type AdminUserListItem = AdminUserListResponse['users'][number]

export interface AdminTeam {
  id: string
  name: string
  slug: string
  description?: string | null
  isDefault: boolean
  createdAt: string
  updatedAt: string
}

export interface AdminTeamMember {
  teamId: string
  userId: string
  role: string
  createdAt: string
}

type UpdateAdminUserRequest = Parameters<typeof updateUserHandler>[1]

interface SaveTeamRequest {
  name: string
  slug?: string
  description?: string | null
}

interface SaveTeamMemberRequest {
  teamId: string
  userId: string
  role: string
}

export function useAdminUsersList() {
  return useQuery({
    queryKey: queryKeys.users.list(),
    queryFn: () => unwrapResponse<AdminUserListResponse>(listUsersHandler()),
  })
}

/**
 * Select a single user from the cached users list. The backend exposes no
 * dedicated get-by-login endpoint, so we derive the user from the list query
 * (which the profile page and the users sidebar already load).
 */
export function useAdminUser(login: string) {
  const query = useAdminUsersList()
  const user = query.data?.users.find((candidate) => candidate.login === login)
  return { ...query, user }
}

export function useAdminTeamsList() {
  return useQuery({
    queryKey: queryKeys.teams.list(),
    queryFn: () => unwrapResponse<AdminTeam[]>(listTeams()),
  })
}

export function useAdminTeamMembers(teamId: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.teams.members(teamId),
    queryFn: () => unwrapResponse<AdminTeamMember[]>(listMembers(teamId)),
    enabled: enabled && !!teamId,
  })
}

export function useAdminTeamMembersByTeam(teams: AdminTeam[]) {
  return useQueries({
    queries: teams.map((team) => ({
      queryKey: queryKeys.teams.members(team.id),
      queryFn: () => unwrapResponse<AdminTeamMember[]>(listMembers(team.id)),
      enabled: teams.length > 0,
    })),
  })
}

export function useUpdateAdminUser() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ login, request }: { login: string; request: UpdateAdminUserRequest }) =>
      unwrapResponse(updateUserHandler(login, request)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.users.list() })
    },
  })
}

export function useSaveAdminTeam() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ teamId, request }: { teamId?: string; request: SaveTeamRequest }) => {
      if (teamId) {
        return unwrapResponse(
          updateTeam(teamId, {
            name: request.name,
            description: request.description ?? null,
          }),
        )
      }
      return unwrapResponse(
        createTeam({
          name: request.name,
          slug: request.slug ?? '',
          description: request.description ?? null,
        }),
      )
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.teams.list() })
    },
  })
}

export function useSaveAdminTeamMember() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ teamId, userId, role }: SaveTeamMemberRequest) =>
      unwrapResponse(addMember(teamId, { userId, role })),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.teams.members(variables.teamId) })
    },
  })
}

export function useRemoveAdminTeamMember() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ teamId, userId }: { teamId: string; userId: string }) =>
      unwrapResponse(removeMember(teamId, userId)),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.teams.members(variables.teamId) })
    },
  })
}
