import type {
  Command,
  CommandList,
  CreateCommandRequest,
  UpdateCommandRequest,
} from '@shared/types/command'
import { apiClient } from '@/lib/api/client'

export const commandsApi = {
  async listCommands(): Promise<Array<Command>> {
    const response = await apiClient.get<CommandList>('/api/v1/commands')
    return response.commands
  },

  async getCommand(commandName: string): Promise<Command> {
    return await apiClient.get<Command>(`/api/v1/commands/${commandName}`)
  },

  async createCommand(request: CreateCommandRequest): Promise<Command> {
    return await apiClient.post<Command>('/api/v1/commands', request)
  },

  async updateCommand(commandName: string, request: UpdateCommandRequest): Promise<Command> {
    return await apiClient.put<Command>(`/api/v1/commands/${commandName}`, request)
  },

  async deleteCommand(commandName: string): Promise<void> {
    await apiClient.delete(`/api/v1/commands/${commandName}`)
  },
}
