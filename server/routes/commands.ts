import type { FastifyInstance } from 'fastify'
import type { z } from 'zod'
import { CommandNotFoundError } from '../../shared/constants/errors.js'
import {
  CommandNameParamsSchema,
  CreateCommandSchema,
  UpdateCommandSchema,
} from '../../shared/schemas/command.schemas.js'
import type { CommandList } from '../../shared/types/command.js'

export async function registerCommandRoutes(app: FastifyInstance) {
  const { command: commandService } = app.services

  // Create command
  app.post(
    '/',
    {
      schema: {
        body: CreateCommandSchema,
      },
    },
    async (request, _reply) => {
      const body = request.body as z.infer<typeof CreateCommandSchema>

      const command = await commandService.createCommand(body)
      return command
    },
  )

  // List commands
  app.get('/', async (_request, _reply) => {
    const commands = await commandService.listCommands()
    return { commands } as CommandList
  })

  // Get command by name
  app.get(
    '/:commandName',
    {
      schema: {
        params: CommandNameParamsSchema,
      },
    },
    async (request, _reply) => {
      const { commandName } = request.params as z.infer<typeof CommandNameParamsSchema>
      const command = await commandService.getCommand(commandName)

      if (!command) {
        throw new CommandNotFoundError(commandName)
      }

      return command
    },
  )

  // Update command
  app.put(
    '/:commandName',
    {
      schema: {
        params: CommandNameParamsSchema,
        body: UpdateCommandSchema,
      },
    },
    async (request, _reply) => {
      const { commandName } = request.params as z.infer<typeof CommandNameParamsSchema>
      const updates = request.body as z.infer<typeof UpdateCommandSchema>

      const updatedCommand = await commandService.updateCommand(commandName, updates)
      return updatedCommand
    },
  )

  // Delete command
  app.delete(
    '/:commandName',
    {
      schema: {
        params: CommandNameParamsSchema,
      },
    },
    async (request, _reply) => {
      const { commandName } = request.params as z.infer<typeof CommandNameParamsSchema>

      await commandService.deleteCommand(commandName)
      return { status: 'deleted' }
    },
  )
}
