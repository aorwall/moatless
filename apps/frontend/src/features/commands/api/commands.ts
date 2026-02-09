import type {
  CommandResponse,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createCommand as createCommandHandler,
  deleteCommand as deleteCommandHandler,
  getCommand as getCommandHandler,
  listCommands as listCommandsHandler,
  updateCommand as updateCommandHandler,
} from '@moatless/api-client/sandbox/commands/commands'

export const commandsApi = {
  async listCommands(): Promise<CommandResponse[]> {
    return await unwrapResponse(listCommandsHandler())
  },

  async getCommand(commandName: string): Promise<CommandResponse> {
    return await unwrapResponse(getCommandHandler(commandName))
  },

  async createCommand(request: CreateCommandRequest): Promise<CommandResponse> {
    return await unwrapResponse(createCommandHandler(request))
  },

  async updateCommand(
    commandName: string,
    request: UpdateCommandRequest,
  ): Promise<CommandResponse> {
    return await unwrapResponse(updateCommandHandler(commandName, request))
  },

  async deleteCommand(commandName: string): Promise<void> {
    await unwrapResponse<void>(deleteCommandHandler(commandName))
  },
}
