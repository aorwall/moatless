import {
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import {
  type CodexConfigResponse,
  type CodexDeviceCodePollRequest,
  type CodexDeviceCodePollResponse,
  type CodexDeviceCodeStartResponse,
  codexConnectionApi,
  type SaveCodexAuthRequest,
} from '@/features/settings/api/codex-connection.api'
import type { ApiError } from '@/lib/api/client'
import { queryKeys } from '@/lib/api/query-keys'

export function useCodexConfig() {
  return useQuery({
    queryKey: queryKeys.settings.codexConfig(),
    queryFn: codexConnectionApi.getConfig,
  })
}

export function useDisconnectCodex(options?: UseMutationOptions<CodexConfigResponse, ApiError>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: codexConnectionApi.disconnect,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.codex() })
    },
    ...options,
  })
}

export function useSaveCodexAuthJson(
  options?: UseMutationOptions<CodexConfigResponse, ApiError, SaveCodexAuthRequest>,
) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: codexConnectionApi.saveAuthJson,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings.codex() })
    },
    ...options,
  })
}

export function useStartCodexDeviceLogin(
  options?: UseMutationOptions<CodexDeviceCodeStartResponse, ApiError, void>,
) {
  return useMutation({
    mutationFn: () => codexConnectionApi.startDeviceLogin(),
    ...options,
  })
}

interface UseCodexDevicePollOptions {
  enabled: boolean
  refetchInterval: number
}

export function useCodexDevicePoll(
  args: CodexDeviceCodePollRequest | null,
  { enabled, refetchInterval }: UseCodexDevicePollOptions,
) {
  return useQuery<CodexDeviceCodePollResponse, ApiError>({
    queryKey: queryKeys.settings.codexDevicePoll(args?.deviceAuthId ?? ''),
    queryFn: () => {
      if (!args) {
        throw new Error('useCodexDevicePoll: args required when enabled')
      }
      return codexConnectionApi.pollDeviceLogin(args)
    },
    enabled: enabled && !!args,
    refetchInterval,
    gcTime: 0,
    staleTime: 0,
  })
}
