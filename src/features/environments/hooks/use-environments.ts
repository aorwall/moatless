import type {
  UseMutationOptions,
  UseQueryOptions,
} from '@tanstack/react-query';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import { environmentsApi } from '@/features/environments/api/environments';
import { queryKeys } from '@/lib/api/query-keys';
import type {
  CreateEnvironmentRequest,
  EnvironmentResponse,
  FileSearchResponse,
  SetupScriptResponse,
  UpdateEnvironmentRequest,
} from '@/types/environment';

// Query hooks
export function useEnvironmentsList(
  options?: Omit<
    UseQueryOptions<Array<EnvironmentResponse>>,
    'queryKey' | 'queryFn'
  >,
) {
  return useQuery({
    queryKey: queryKeys.environmentsList(),
    queryFn: () => environmentsApi.listEnvironments(),
    ...options,
  });
}

export function useEnvironment(
  envId: string,
  options?: Omit<UseQueryOptions<EnvironmentResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.environmentDetail(envId),
    queryFn: () => environmentsApi.getEnvironment(envId),
    enabled: !!envId,
    ...options,
  });
}

// Mutation hooks
export function useCreateEnvironment(
  options?: UseMutationOptions<
    EnvironmentResponse,
    Error,
    CreateEnvironmentRequest
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateEnvironmentRequest) =>
      environmentsApi.createEnvironment(request),
    onSuccess: (data, variables, context) => {
      // Invalidate environments list to refresh with new environment
      queryClient.invalidateQueries({ queryKey: queryKeys.environments() });
      options?.onSuccess?.(data, variables, context);
    },
    ...options,
  });
}

export function useUpdateEnvironment(
  options?: UseMutationOptions<
    EnvironmentResponse,
    Error,
    { envId: string; request: UpdateEnvironmentRequest }
  >,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ envId, request }) =>
      environmentsApi.updateEnvironment(envId, request),
    onSuccess: (data, variables, context) => {
      // Invalidate specific environment detail
      queryClient.invalidateQueries({
        queryKey: queryKeys.environmentDetail(variables.envId),
      });
      // Invalidate environments list
      queryClient.invalidateQueries({ queryKey: queryKeys.environments() });
      options?.onSuccess?.(data, variables, context);
    },
    ...options,
  });
}

export function useDeleteEnvironment(
  options?: UseMutationOptions<void, Error, string>,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (envId: string) => environmentsApi.deleteEnvironment(envId),
    onSuccess: (data, envId, context) => {
      // Remove from cache
      queryClient.removeQueries({
        queryKey: queryKeys.environmentDetail(envId),
      });
      // Invalidate environments list
      queryClient.invalidateQueries({ queryKey: queryKeys.environments() });
      options?.onSuccess?.(data, envId, context);
    },
    ...options,
  });
}

export function useRunSetupScript(
  options?: UseMutationOptions<SetupScriptResponse, Error, string>,
) {
  return useMutation({
    mutationFn: (envId: string) => environmentsApi.runSetupScript(envId),
    ...options,
  });
}

export function useSearchFiles(
  envId: string,
  query: string,
  options?: Omit<UseQueryOptions<FileSearchResponse>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.environmentFileSearch(envId, query),
    queryFn: () => environmentsApi.searchFiles(envId, { query, limit: 10 }),
    enabled: !!envId && query.length >= 3,
    staleTime: 30000, // Cache for 30 seconds
    ...options,
  });
}
