import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  createCommandRoute,
  deleteCommandRoute,
  getCommandRoute,
  listCommandsRoute,
  updateCommandRoute,
} from '@moatless/api-contract/commands'
import type { CommandList } from '@moatless/types'
import { CommandNotFoundError } from '@moatless/types'
import type { AppBindings } from '../app'
import { appContainer } from '../container/container'
import { CommandService } from '../features/agents/services/command.service'

export function registerCommandRoutes(app: OpenAPIHono<AppBindings>) {
  // Create command
  app.openapi(createCommandRoute, async (c) => {
    const commandService = appContainer.get(CommandService)
    const validatedBody = c.req.valid('json')

    const command = await commandService.createCommand(validatedBody)
    return c.json(command)
  })

  // List commands
  app.openapi(listCommandsRoute, async (c) => {
    const commandService = appContainer.get(CommandService)

    const commands = await commandService.listCommands()
    return c.json({ commands } as CommandList)
  })

  // Get command by name
  app.openapi(getCommandRoute, async (c) => {
    const commandService = appContainer.get(CommandService)
    const { commandName } = c.req.valid('param')

    const command = await commandService.getCommand(commandName)
    if (!command) {
      throw new CommandNotFoundError(commandName)
    }

    return c.json(command)
  })

  // Update command
  app.openapi(updateCommandRoute, async (c) => {
    const commandService = appContainer.get(CommandService)
    const { commandName } = c.req.valid('param')
    const validatedBody = c.req.valid('json')

    const updatedCommand = await commandService.updateCommand(commandName, validatedBody)
    return c.json(updatedCommand)
  })

  // Delete command
  app.openapi(deleteCommandRoute, async (c) => {
    const commandService = appContainer.get(CommandService)
    const { commandName } = c.req.valid('param')

    await commandService.deleteCommand(commandName)
    return c.json({ status: 'deleted' })
  })
}
