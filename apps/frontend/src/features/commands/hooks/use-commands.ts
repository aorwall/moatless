import type {
  Command,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@moatless/types/types/command'
import type { UseMutationOptions, UseQueryOptions } from '@tanstack/react-query'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { commandsApi } from '@/features/commands/api/commands'
import { queryKeys } from '@/lib/api/query-keys'

// Query hooks
export function useCommandsList(
  options?: Omit<UseQueryOptions<Array<Command>>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.commandsList(),
    queryFn: () => commandsApi.listCommands(),
    ...options,
  })
}

export function useCommand(
  commandName: string,
  options?: Omit<UseQueryOptions<Command>, 'queryKey' | 'queryFn'>,
) {
  return useQuery({
    queryKey: queryKeys.commandDetail(commandName),
    queryFn: () => commandsApi.getCommand(commandName),
    enabled: !!commandName,
    ...options,
  })
}

// Mutation hooks
export function useCreateCommand(
  options?: UseMutationOptions<Command, Error, CreateCommandRequest>,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (request: CreateCommandRequest) => commandsApi.createCommand(request),
    onSuccess: (data, variables, context) => {
      // Invalidate commands list to refresh with new command
      queryClient.invalidateQueries({ queryKey: queryKeys.commands() })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useUpdateCommand(
  options?: UseMutationOptions<
    Command,
    Error,
    { commandName: string; request: UpdateCommandRequest }
  >,
) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: ({ commandName, request }) => commandsApi.updateCommand(commandName, request),
    onSuccess: (data, variables, context) => {
      // Invalidate specific command detail
      queryClient.invalidateQueries({
        queryKey: queryKeys.commandDetail(variables.commandName),
      })
      // Invalidate commands list
      queryClient.invalidateQueries({ queryKey: queryKeys.commands() })
      options?.onSuccess?.(data, variables, context)
    },
    ...options,
  })
}

export function useDeleteCommand(options?: UseMutationOptions<void, Error, string>) {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: (commandName: string) => commandsApi.deleteCommand(commandName),
    onSuccess: (data, commandName, context) => {
      // Remove from cache
      queryClient.removeQueries({
        queryKey: queryKeys.commandDetail(commandName),
      })
      // Invalidate commands list
      queryClient.invalidateQueries({ queryKey: queryKeys.commands() })
      options?.onSuccess?.(data, commandName, context)
    },
    ...options,
  })
}
