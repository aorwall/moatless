import {
  buildPath,
  type RouteParams,
  type RouteRequestJson,
  type RouteSuccess,
} from '@moatless/api-contract'
import {
  createCommandRoute,
  deleteCommandRoute,
  getCommandRoute,
  listCommandsRoute,
  updateCommandRoute,
} from '@moatless/api-contract/commands.ts'
import { apiClient } from '@/lib/api/client'

type CommandListResponse = RouteSuccess<typeof listCommandsRoute>
type Command = RouteSuccess<typeof getCommandRoute>
type CreateCommandRequest = RouteRequestJson<typeof createCommandRoute>
type UpdateCommandRequest = RouteRequestJson<typeof updateCommandRoute>

export const commandsApi = {
  async listCommands(): Promise<Command[]> {
    const path = buildPath(listCommandsRoute)
    const response = await apiClient.get<CommandListResponse>(path)
    return response.commands
  },

  async getCommand(
    commandName: RouteParams<typeof getCommandRoute>['commandName'],
  ): Promise<Command> {
    const path = buildPath(getCommandRoute, { commandName })
    return await apiClient.get<Command>(path)
  },

  async createCommand(request: CreateCommandRequest): Promise<Command> {
    const path = buildPath(createCommandRoute)
    return await apiClient.post<Command>(path, request)
  },

  async updateCommand(
    commandName: RouteParams<typeof updateCommandRoute>['commandName'],
    request: UpdateCommandRequest,
  ): Promise<Command> {
    const path = buildPath(updateCommandRoute, { commandName })
    return await apiClient.put<Command>(path, request)
  },

  async deleteCommand(
    commandName: RouteParams<typeof deleteCommandRoute>['commandName'],
  ): Promise<void> {
    const path = buildPath(deleteCommandRoute, { commandName })
    await apiClient.delete(path)
  },
}
