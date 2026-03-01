import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type {
  CreateSlackChannelMappingRequest,
  SlackChannelMappingResponse,
} from '@/lib/types/slack-channel-mappings'
import { slackChannelMappingsApi } from '../api/slack-channel-mappings'

// Query keys for Slack channel mappings
export const slackMappingKeys = {
  all: ['slack-channel-mappings'] as const,
  lists: () => [...slackMappingKeys.all, 'list'] as const,
  forRepository: (repositoryId: string) =>
    [...slackMappingKeys.all, 'repository', repositoryId] as const,
}

/**
 * Hook to list all Slack channel mappings.
 */
export function useSlackChannelMappings(
  options?: Omit<UseQueryOptions<SlackChannelMappingResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: slackMappingKeys.lists(),
    queryFn: () => slackChannelMappingsApi.listMappings(),
    ...options,
  })
}

/**
 * Hook to get Slack channel mappings for a specific repository.
 */
export function useSlackChannelMappingsForRepository(
  repositoryId: string,
  options?: Omit<UseQueryOptions<SlackChannelMappingResponse[]>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: slackMappingKeys.forRepository(repositoryId),
    queryFn: () => slackChannelMappingsApi.getMappingsForRepository(repositoryId),
    enabled: !!repositoryId,
    ...options,
  })
}

/**
 * Hook to create a new Slack channel mapping.
 */
export function useCreateSlackChannelMapping(
  options?: UseMutationOptions<
    SlackChannelMappingResponse,
    Error,
    CreateSlackChannelMappingRequest
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateSlackChannelMappingRequest) =>
      slackChannelMappingsApi.createMapping(request),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate all mappings lists
      queryClient.invalidateQueries({ queryKey: slackMappingKeys.all })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}

/**
 * Hook to delete a Slack channel mapping.
 */
export function useDeleteSlackChannelMapping(
  options?: UseMutationOptions<void, Error, { channelId: string; teamId: string }>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ channelId, teamId }) => slackChannelMappingsApi.deleteMapping(channelId, teamId),
    onSuccess: (data, variables, onMutateResult, mutationContext) => {
      // Invalidate all mappings lists
      queryClient.invalidateQueries({ queryKey: slackMappingKeys.all })
      options?.onSuccess?.(data, variables, onMutateResult, mutationContext)
    },
    ...options,
  })
}
