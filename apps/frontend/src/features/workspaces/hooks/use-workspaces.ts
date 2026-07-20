import type {
  AddRepoRequest,
  CreateWorkspaceRequest,
  UpdateWorkspaceRequest,
  WorkspaceResponse,
} from '@moatless/api-client'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { workspacesApi } from '@/features/workspaces/api/workspaces'
import { queryKeys } from '@/lib/api/query-keys'

// Query hooks

/** List all workspaces visible to the current user. */
export function useWorkspacesList(
  options?: Omit<UseQueryOptions<WorkspaceResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.workspaces.list(),
    queryFn: () => workspacesApi.listWorkspaces(),
    ...options,
  })
}

/** Fetch a single workspace, including its repository placements. */
export function useWorkspace(
  workspaceId: string,
  options?: Omit<UseQueryOptions<WorkspaceResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.workspaces.detail(workspaceId),
    queryFn: () => workspacesApi.getWorkspace(workspaceId),
    enabled: !!workspaceId,
    ...options,
  })
}

// Mutation hooks

export function useCreateWorkspace(
  options?: UseMutationOptions<WorkspaceResponse, Error, CreateWorkspaceRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateWorkspaceRequest) => workspacesApi.createWorkspace(request),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.all() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useUpdateWorkspace(
  options?: UseMutationOptions<
    WorkspaceResponse,
    Error,
    { workspaceId: string; request: UpdateWorkspaceRequest }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ workspaceId, request }) => workspacesApi.updateWorkspace(workspaceId, request),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.workspaces.detail(variables.workspaceId), data)
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.list() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useDeleteWorkspace(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (workspaceId: string) => workspacesApi.deleteWorkspace(workspaceId),
    onSuccess: (data, workspaceId, onMutateResult, mutationContext) => {
      queryClient.removeQueries({ queryKey: queryKeys.workspaces.detail(workspaceId) })
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.list() })
      options?.onSuccess?.(data, workspaceId, onMutateResult, mutationContext)
    },
    ...options,
  })
}

// Composition mutations — each returns the updated workspace, so seed the
// detail cache directly and refresh the list (kind label may have changed).

export function useAddWorkspaceRepo(
  options?: UseMutationOptions<
    WorkspaceResponse,
    Error,
    { workspaceId: string; request: AddRepoRequest }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ workspaceId, request }) => workspacesApi.addRepo(workspaceId, request),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.workspaces.detail(variables.workspaceId), data)
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.list() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useRemoveWorkspaceRepo(
  options?: UseMutationOptions<
    WorkspaceResponse,
    Error,
    { workspaceId: string; placementId: string }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ workspaceId, placementId }) =>
      workspacesApi.removeRepo(workspaceId, placementId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.workspaces.detail(variables.workspaceId), data)
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.list() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

export function useSetWorkspacePrimaryRepo(
  options?: UseMutationOptions<
    WorkspaceResponse,
    Error,
    { workspaceId: string; placementId: string }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ workspaceId, placementId }) =>
      workspacesApi.setPrimaryRepo(workspaceId, placementId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      queryClient.setQueryData(queryKeys.workspaces.detail(variables.workspaceId), data)
      queryClient.invalidateQueries({ queryKey: queryKeys.workspaces.list() })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}
