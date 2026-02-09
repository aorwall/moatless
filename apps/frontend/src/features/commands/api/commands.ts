import type { CommandResponse } from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  createCommand as createCommandHandler,
  deleteCommand as deleteCommandHandler,
  getCommand as getCommandHandler,
  listCommands as listCommandsHandler,
  updateCommand as updateCommandHandler,
} from '@moatless/api-client/sandbox/commands/commands'
import type { Command, CreateCommandRequest, UpdateCommandRequest } from '@moatless/types'

export const commandsApi = {
  async listCommands(): Promise<Command[]> {
    const result = await unwrapResponse<CommandResponse[]>(listCommandsHandler())
    return result as unknown as Command[]
  },

  async getCommand(commandName: string): Promise<Command> {
    const result = await unwrapResponse<CommandResponse>(getCommandHandler(commandName))
    return result as unknown as Command
  },

  async createCommand(request: CreateCommandRequest): Promise<Command> {
    const result = await unwrapResponse<CommandResponse>(
      createCommandHandler(request as unknown as Parameters<typeof createCommandHandler>[0]),
    )
    return result as unknown as Command
  },

  async updateCommand(commandName: string, request: UpdateCommandRequest): Promise<Command> {
    const result = await unwrapResponse<CommandResponse>(
      updateCommandHandler(
        commandName,
        request as unknown as Parameters<typeof updateCommandHandler>[1],
      ),
    )
    return result as unknown as Command
  },

  async deleteCommand(commandName: string): Promise<void> {
    await unwrapResponse<void>(deleteCommandHandler(commandName))
  },
}
