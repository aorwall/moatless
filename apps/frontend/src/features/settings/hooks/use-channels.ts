import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  type ChannelConfigResponse,
  type CreateChannelConfigRequest,
  channelsApi,
  type PairingRequestResponse,
  type SlackChannelInfo,
  type UpdateChannelConfigRequest,
} from '@/features/settings/api/channels.api'
import { queryKeys } from '@/lib/api/query-keys'

// Re-export types for consumers
export type {
  ChannelConfigResponse,
  CreateChannelConfigRequest,
  PairingRequestResponse,
  SlackChannelInfo,
}

/**
 * List configured channel providers (e.g. ['slack', 'telegram']).
 * Long staleTime since providers don't change at runtime.
 */
export function useChannelProviders() {
  return useQuery({
    queryKey: queryKeys.channels.providers(),
    queryFn: () => channelsApi.listProviders(),
    staleTime: 5 * 60_000,
  })
}

/**
 * List all channel configs.
 */
export function useChannelConfigs() {
  return useQuery({
    queryKey: queryKeys.channels.configsList(),
    queryFn: () => channelsApi.listConfigs(),
  })
}

/**
 * List available Slack channels the bot is a member of.
 * Cached for 60s to avoid Slack API rate limits.
 */
export function useSlackChannels(options?: { enabled?: boolean }) {
  return useQuery({
    queryKey: queryKeys.channels.slackChannels(),
    queryFn: () => channelsApi.listSlackChannels(),
    staleTime: 60_000,
    enabled: options?.enabled,
  })
}

/**
 * Create a new channel config.
 */
export function useCreateChannelConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (request: CreateChannelConfigRequest) => channelsApi.createConfig(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.channels.configs() })
    },
  })
}

/**
 * Update a channel config.
 */
export function useUpdateChannelConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, request }: { id: string; request: UpdateChannelConfigRequest }) =>
      channelsApi.updateConfig(id, request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.channels.configs() })
    },
  })
}

/**
 * Delete a channel config.
 */
export function useDeleteChannelConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => channelsApi.deleteConfig(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.channels.configs() })
    },
  })
}

/**
 * List all pending pairing requests across the user's channels.
 * Polls every 30s.
 */
export function useMyPairingRequests() {
  return useQuery({
    queryKey: queryKeys.channels.pairingRequests(),
    queryFn: () => channelsApi.listMyPairingRequests(),
    refetchInterval: 30_000,
  })
}

/**
 * Approve a pairing request by config ID + code.
 */
export function useApprovePairing() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ configId, code }: { configId: string; code: string }) =>
      channelsApi.approvePairing(configId, code),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.channels.pairingRequests() })
    },
  })
}
